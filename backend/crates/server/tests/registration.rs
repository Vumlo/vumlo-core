//! ADR-0046: self-registration, email verification and password reset, with the mail delivered by
//! the outbox worker (ADR-0045) into a directory.

mod common;

use ::common::Role;
use common::{Http, Mailbox, PASSWORD, db_test, seed_and_login, seed_user, spawn_app};
use server::events;
use sqlx::PgPool;

const NEW_PASSWORD: &str = "a different sturdy passphrase 7";

/// Delivers the outbox with the handlers of the running app.
async fn deliver(state: &server::app::AppState) {
    events::process_once(&state.db, &state.event_handlers())
        .await
        .expect("deliver");
}

fn register_body(email: &str) -> serde_json::Value {
    serde_json::json!({"name": "Maya", "surname": "Okafor", "email": email, "password": PASSWORD})
}

async fn login(http: &Http, email: &str, password: &str) -> common::Resp {
    http.post("/auth/login")
        .json(&serde_json::json!({"email": email, "password": password, "client": "app"}))
        .send()
        .await
}

async fn outbox_types(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar!("SELECT type FROM events.outbox ORDER BY id")
        .fetch_all(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_new_address_gets_a_mail_and_can_log_in_only_after_the_link() {
    db_test(async |pool| {
        let mailbox = Mailbox::new();
        let (base, state) = spawn_app(pool.clone(), mailbox.settings()).await;
        let http = Http::new(&base);

        let response = http
            .post("/auth/register")
            .json(&register_body("Maya@LMS.test"))
            .send()
            .await;

        assert_eq!(response.status, 202);
        assert!(response.body.is_empty());
        let row = sqlx::query!(
            "SELECT role, status, must_change_password, email_verified_at FROM auth.users
             WHERE email = 'maya@lms.test'"
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.role, "student");
        assert_eq!(row.status, "active");
        assert!(!row.must_change_password);
        assert!(row.email_verified_at.is_none());
        let event = sqlx::query!("SELECT type, payload FROM events.outbox")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(event.r#type, "user.verification_requested");
        assert_eq!(
            event
                .payload
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            ["user_id"],
            "ids only (ADR-0007)"
        );
        assert!(
            mailbox.messages().is_empty(),
            "nothing is sent inside the request"
        );

        deliver(&state).await;

        let messages = mailbox.messages();
        assert_eq!(messages.len(), 1);
        for needle in [
            "maya@lms.test",
            "Subject: Confirm your email address",
            "Test University",
        ] {
            assert!(messages[0].contains(needle), "{needle}: {}", messages[0]);
        }
        let token = Mailbox::token_in(&messages[0]);
        assert!(
            messages[0].contains(&format!("https://lms.test/verify-email?token={token}")),
            "{}",
            messages[0]
        );
        let stored = sqlx::query!(
            "SELECT purpose, token_hash, used_at,
                    expires_at - created_at = interval '24 hours' AS \"is_a_day!\"
             FROM auth.email_tokens"
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(stored.purpose, "verify");
        assert_eq!(stored.token_hash.len(), 32, "a hash, not the token");
        assert!(
            !stored
                .token_hash
                .windows(token.len())
                .any(|w| w == token.as_bytes())
        );
        assert!(stored.used_at.is_none());
        assert!(stored.is_a_day);
        let processed =
            sqlx::query_scalar!(r#"SELECT processed_at IS NOT NULL AS "done!" FROM events.outbox"#)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(processed);

        // The right password still does not log in before the address is verified.
        let early = login(&http, "maya@lms.test", PASSWORD).await;
        early.problem(403);
        let wrong = login(&http, "maya@lms.test", "wrong password").await;
        assert_eq!(
            wrong.status, 401,
            "a wrong password learns nothing about verification"
        );

        let verified = http
            .post("/auth/verify-email")
            .json(&serde_json::json!({"token": token}))
            .send()
            .await;
        assert_eq!(verified.status, 204);
        assert_eq!(login(&http, "maya@lms.test", PASSWORD).await.status, 200);

        // A link works once.
        http.post("/auth/verify-email")
            .json(&serde_json::json!({"token": token}))
            .send()
            .await
            .problem(410);
    })
    .await;
}

#[tokio::test]
async fn registration_answers_the_same_for_every_address() {
    db_test(async |pool| {
        let mailbox = Mailbox::new();
        let (base, state) = spawn_app(pool.clone(), mailbox.settings()).await;
        let http = Http::new(&base);
        seed_user(&pool, "verified@lms.test", Role::Student).await;
        seed_user(&pool, "teacher@lms.test", Role::Teacher).await;
        seed_user(&pool, "off@lms.test", Role::Student).await;
        sqlx::query!("UPDATE auth.users SET status = 'disabled' WHERE email = 'off@lms.test'")
            .execute(&pool)
            .await
            .unwrap();
        // An earlier registration that is still unverified.
        http.post("/auth/register")
            .json(&register_body("pending@lms.test"))
            .send()
            .await;
        sqlx::query!("DELETE FROM events.outbox")
            .execute(&pool)
            .await
            .unwrap();

        let mut answers = Vec::new();
        for email in [
            "fresh@lms.test",
            "verified@lms.test",
            "teacher@lms.test",
            "off@lms.test",
            "pending@lms.test",
        ] {
            let response = http
                .post("/auth/register")
                .json(&register_body(email))
                .send()
                .await;
            answers.push((
                response.status,
                response.body.clone(),
                response.headers.get("content-type").cloned(),
            ));
        }

        assert!(answers.iter().all(|a| *a == answers[0]), "{answers:?}");
        assert_eq!(answers[0].0, 202);
        // Only the new address and the unverified one cause a mail.
        let events = sqlx::query_scalar!(
            r#"SELECT (payload->>'user_id')::uuid AS "id!" FROM events.outbox"#
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let mut emailed = Vec::new();
        for id in events {
            emailed.push(
                sqlx::query_scalar!("SELECT email FROM auth.users WHERE id = $1", id)
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
            );
        }
        emailed.sort();
        assert_eq!(emailed, ["fresh@lms.test", "pending@lms.test"]);
        deliver(&state).await;
        // The existing accounts are untouched: same names, still verified, same role.
        let teacher =
            sqlx::query!("SELECT name, role FROM auth.users WHERE email = 'teacher@lms.test'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            (teacher.name.as_str(), teacher.role.as_str()),
            ("Test", "teacher")
        );
        assert_eq!(
            login(&http, "verified@lms.test", PASSWORD).await.status,
            200
        );
    })
    .await;
}

#[tokio::test]
async fn a_registration_is_checked_before_anything_is_created() {
    db_test(async |pool| {
        let mailbox = Mailbox::new();
        let (base, _) = spawn_app(pool.clone(), mailbox.settings()).await;
        let http = Http::new(&base);

        let foreign = http
            .post("/auth/register")
            .json(&register_body("maya@elsewhere.test"))
            .send()
            .await
            .problem(422);
        assert_eq!(foreign.errors[0].field, "email");

        let mut weak = register_body("weak@lms.test");
        weak["password"] = "password1234".into();
        let weak = http
            .post("/auth/register")
            .json(&weak)
            .send()
            .await
            .problem(422);
        assert!(
            weak.errors.iter().all(|e| e.field == "password"),
            "{weak:?}"
        );
        assert!(!weak.errors.is_empty());

        let mut bad_name = register_body("name@lms.test");
        bad_name["name"] = "   ".into();
        let bad_name = http
            .post("/auth/register")
            .json(&bad_name)
            .send()
            .await
            .problem(422);
        assert_eq!(bad_name.errors[0].field, "name");

        let users = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM auth.users"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(users, 0);
        assert!(outbox_types(&pool).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn registration_and_reset_are_off_without_mail_and_no_domain_means_no_sign_up() {
    db_test(async |pool| {
        let (base, _) = spawn_app(pool.clone(), common::test_settings()).await;
        let http = Http::new(&base);
        http.post("/auth/register")
            .json(&register_body("maya@lms.test"))
            .send()
            .await
            .problem(404);
        http.post("/auth/password-reset")
            .json(&serde_json::json!({"email": "maya@lms.test"}))
            .send()
            .await
            .problem(404);

        let mailbox = Mailbox::new();
        let mut settings = mailbox.settings();
        settings.auth.allowed_domains.clear();
        let (base, _) = spawn_app(pool, settings).await;
        Http::new(&base)
            .post("/auth/register")
            .json(&register_body("maya@lms.test"))
            .send()
            .await
            .problem(422);
    })
    .await;
}

#[tokio::test]
async fn registering_again_replaces_the_password_and_voids_the_earlier_link() {
    db_test(async |pool| {
        let mailbox = Mailbox::new();
        let (base, state) = spawn_app(pool.clone(), mailbox.settings()).await;
        let http = Http::new(&base);

        // A stranger registers the victim's address with a password of their own.
        let mut stranger = register_body("maya@lms.test");
        stranger["password"] = "the stranger chose this one 55".into();
        stranger["name"] = "Stranger".into();
        http.post("/auth/register").json(&stranger).send().await;
        deliver(&state).await;
        let stranger_link = Mailbox::token_in(&mailbox.messages()[0]);

        // The owner registers: the password and the name are theirs now, the first link is dead.
        http.post("/auth/register")
            .json(&register_body("maya@lms.test"))
            .send()
            .await;
        deliver(&state).await;
        let messages = mailbox.messages();
        assert_eq!(messages.len(), 2);
        let owner_link = Mailbox::token_in(&messages[1]);
        assert_ne!(owner_link, stranger_link);
        http.post("/auth/verify-email")
            .json(&serde_json::json!({"token": stranger_link}))
            .send()
            .await
            .problem(410);
        assert_eq!(
            http.post("/auth/verify-email")
                .json(&serde_json::json!({"token": owner_link}))
                .send()
                .await
                .status,
            204
        );
        assert_eq!(login(&http, "maya@lms.test", PASSWORD).await.status, 200);
        assert_eq!(
            login(&http, "maya@lms.test", "the stranger chose this one 55")
                .await
                .status,
            401
        );
        let name = sqlx::query_scalar!("SELECT name FROM auth.users WHERE email = 'maya@lms.test'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(name, "Maya");
    })
    .await;
}

#[tokio::test]
async fn an_expired_or_malformed_verification_link_is_gone() {
    db_test(async |pool| {
        let mailbox = Mailbox::new();
        let (base, state) = spawn_app(pool.clone(), mailbox.settings()).await;
        let http = Http::new(&base);
        http.post("/auth/register")
            .json(&register_body("maya@lms.test"))
            .send()
            .await;
        deliver(&state).await;
        let token = Mailbox::token_in(&mailbox.messages()[0]);
        sqlx::query!(
            "UPDATE auth.email_tokens
             SET created_at = now() - interval '25 hours', expires_at = now() - interval '1 hour'"
        )
        .execute(&pool)
        .await
        .unwrap();

        for bad in [
            token.as_str(),
            "short",
            "not a token at all, with spaces and punctuation!!!!",
        ] {
            let response = http
                .post("/auth/verify-email")
                .json(&serde_json::json!({"token": bad}))
                .send()
                .await;
            assert_eq!(response.status, 410, "{bad}");
        }
        assert_eq!(login(&http, "maya@lms.test", PASSWORD).await.status, 403);
    })
    .await;
}

#[tokio::test]
async fn a_password_reset_sets_the_password_ends_every_session_and_works_once() {
    db_test(async |pool| {
        let mailbox = Mailbox::new();
        let (base, state) = spawn_app(pool.clone(), mailbox.settings()).await;
        let http = Http::new(&base);
        let (_, session) = seed_and_login(&pool, &http, "maya@lms.test", Role::Student).await;

        let response = http
            .post("/auth/password-reset")
            .json(&serde_json::json!({"email": "Maya@lms.test"}))
            .send()
            .await;
        assert_eq!(response.status, 202);
        assert!(response.body.is_empty());
        deliver(&state).await;
        let messages = mailbox.messages();
        assert_eq!(messages.len(), 1);
        assert!(
            messages[0].contains("Subject: Reset your password"),
            "{}",
            messages[0]
        );
        let token = Mailbox::token_in(&messages[0]);
        assert!(messages[0].contains(&format!("https://lms.test/reset-password?token={token}")));

        // The link is checked before the password: a spent token is 410 whatever the password is.
        http.post("/auth/password-reset/confirm")
            .json(&serde_json::json!({"token": "x".repeat(43), "new_password": "1"}))
            .send()
            .await
            .problem(410);
        // A weak password is 422 and leaves the link usable.
        let weak = http
            .post("/auth/password-reset/confirm")
            .json(&serde_json::json!({"token": token, "new_password": "password1234"}))
            .send()
            .await
            .problem(422);
        assert!(
            weak.errors.iter().all(|e| e.field == "new_password"),
            "{weak:?}"
        );

        let done = http
            .post("/auth/password-reset/confirm")
            .json(&serde_json::json!({"token": token, "new_password": NEW_PASSWORD}))
            .send()
            .await;
        assert_eq!(done.status, 204);

        assert_eq!(
            http.get("/me").bearer(&session).send().await.status,
            401,
            "sessions end"
        );
        assert_eq!(login(&http, "maya@lms.test", PASSWORD).await.status, 401);
        assert_eq!(
            login(&http, "maya@lms.test", NEW_PASSWORD).await.status,
            200
        );
        http.post("/auth/password-reset/confirm")
            .json(&serde_json::json!({"token": token, "new_password": NEW_PASSWORD}))
            .send()
            .await
            .problem(410);
    })
    .await;
}

#[tokio::test]
async fn a_reset_asks_nothing_about_unknown_or_disabled_addresses_and_verifies_the_mailbox() {
    db_test(async |pool| {
        let mailbox = Mailbox::new();
        let (base, state) = spawn_app(pool.clone(), mailbox.settings()).await;
        let http = Http::new(&base);
        seed_user(&pool, "off@lms.test", Role::Student).await;
        sqlx::query!("UPDATE auth.users SET status = 'disabled' WHERE email = 'off@lms.test'")
            .execute(&pool)
            .await
            .unwrap();
        http.post("/auth/register")
            .json(&register_body("pending@lms.test"))
            .send()
            .await;
        deliver(&state).await;
        sqlx::query!("DELETE FROM events.outbox")
            .execute(&pool)
            .await
            .unwrap();

        let mut answers = Vec::new();
        for email in [
            "nobody@lms.test",
            "off@lms.test",
            "pending@lms.test",
            "not an email",
        ] {
            let response = http
                .post("/auth/password-reset")
                .json(&serde_json::json!({"email": email}))
                .send()
                .await;
            answers.push((response.status, response.body));
        }
        assert!(answers.iter().all(|a| *a == answers[0]), "{answers:?}");
        assert_eq!(answers[0].0, 202);
        assert_eq!(
            outbox_types(&pool).await,
            ["user.password_reset_requested"],
            "only the active account causes a mail"
        );

        // The unverified account proves its mailbox by using the reset link.
        deliver(&state).await;
        let token = Mailbox::token_in(mailbox.messages().last().unwrap());
        http.post("/auth/password-reset/confirm")
            .json(&serde_json::json!({"token": token, "new_password": NEW_PASSWORD}))
            .send()
            .await;
        assert_eq!(
            login(&http, "pending@lms.test", NEW_PASSWORD).await.status,
            200
        );
    })
    .await;
}

#[tokio::test]
async fn the_link_of_an_account_disabled_since_is_gone() {
    db_test(async |pool| {
        let mailbox = Mailbox::new();
        let (base, state) = spawn_app(pool.clone(), mailbox.settings()).await;
        let http = Http::new(&base);
        seed_user(&pool, "maya@lms.test", Role::Student).await;
        http.post("/auth/password-reset")
            .json(&serde_json::json!({"email": "maya@lms.test"}))
            .send()
            .await;
        deliver(&state).await;
        let token = Mailbox::token_in(&mailbox.messages()[0]);
        sqlx::query!("UPDATE auth.users SET status = 'disabled'")
            .execute(&pool)
            .await
            .unwrap();

        http.post("/auth/password-reset/confirm")
            .json(&serde_json::json!({"token": token, "new_password": NEW_PASSWORD}))
            .send()
            .await
            .problem(410);
    })
    .await;
}

#[tokio::test]
async fn requests_that_send_mail_are_counted_per_address() {
    db_test(async |pool| {
        let mailbox = Mailbox::new();
        let mut settings = mailbox.settings();
        settings.auth.login_account_max_attempts = 3;
        let (base, _) = spawn_app(pool, settings).await;
        let http = Http::new(&base);

        for _ in 0..3 {
            let response = http
                .post("/auth/password-reset")
                .json(&serde_json::json!({"email": "maya@lms.test"}))
                .send()
                .await;
            assert_eq!(response.status, 202);
        }
        let refused = http
            .post("/auth/password-reset")
            .json(&serde_json::json!({"email": "MAYA@lms.test"}))
            .send()
            .await;
        refused.problem(429);
        assert!(refused.headers.contains_key("retry-after"));
        // Another address has its own allowance, and registering counts separately from login.
        assert_eq!(
            http.post("/auth/password-reset")
                .json(&serde_json::json!({"email": "other@lms.test"}))
                .send()
                .await
                .status,
            202
        );
    })
    .await;
}

#[tokio::test]
async fn an_account_made_by_an_admin_can_log_in_at_once() {
    db_test(async |pool| {
        let (base, _) = spawn_app(pool.clone(), common::test_settings()).await;
        let http = Http::new(&base);
        let (_, admin) = seed_and_login(&pool, &http, "admin@lms.test", Role::Admin).await;

        let created = http
            .post("/admin/users")
            .bearer(&admin)
            .json(&serde_json::json!({"email": "new@lms.test", "name": "N", "surname": "N", "role": "teacher"}))
            .send()
            .await;
        assert_eq!(created.status, 201);

        let temporary = created.json()["temporary_password"].as_str().unwrap().to_owned();
        assert_eq!(login(&http, "new@lms.test", &temporary).await.status, 200);
    })
    .await;
}
