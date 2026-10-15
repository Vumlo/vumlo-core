mod common;

use common::{Http, ORIGIN, PASSWORD, login_app, login_web, seed_pending_user, seed_user, spawn};
use serde_json::json;
use server::identity::Role;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

async fn setup(pool: &PgPool) -> Http {
    seed_user(pool, "anna@school.edu", Role::Student).await;
    Http::new(&spawn(pool.clone()).await)
}

fn login_body(email: &str, password: &str, client: &str) -> serde_json::Value {
    json!({"email": email, "password": password, "client": client})
}

// --- login -----------------------------------------------------------------------------------

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn login_web_sets_session_cookie(pool: PgPool) {
    let http = setup(&pool).await;

    let response = http
        .post("/auth/login")
        .origin(ORIGIN)
        .json(&login_body("Anna@School.edu", PASSWORD, "web"))
        .send()
        .await;

    assert_eq!(response.status, 200);
    let cookie = response.header("set-cookie");
    for attribute in ["HttpOnly", "SameSite=Lax", "Path=/", "Secure", "Max-Age="] {
        assert!(
            cookie.contains(attribute),
            "{attribute} missing in {cookie}"
        );
    }
    let body = response.json();
    assert_eq!(body["email"], "anna@school.edu");
    assert_eq!(body["role"], "student");
    assert!(
        body.get("token").is_none(),
        "the web token only travels in the cookie"
    );
    assert_eq!(response.header("cache-control"), "no-store");
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn login_app_returns_bearer_token(pool: PgPool) {
    let http = setup(&pool).await;

    let response = http
        .post("/auth/login")
        .json(&login_body("anna@school.edu", PASSWORD, "app"))
        .send()
        .await;

    assert_eq!(response.status, 200);
    assert!(response.headers.get("set-cookie").is_none());
    assert_eq!(response.header("cache-control"), "no-store");
    let body = response.json();
    let token = body["token"].as_str().unwrap();
    assert_eq!(token.len(), 43);
    assert_eq!(body["user"]["email"], "anna@school.edu");
    let expires =
        chrono::DateTime::parse_from_rfc3339(body["expires_at"].as_str().unwrap()).unwrap();
    let days = (expires.to_utc() - chrono::Utc::now()).num_days();
    assert!(
        (89..=90).contains(&days),
        "app sessions last 90 days, got {days}"
    );

    // The token works as a bearer credential: logging out succeeds once, then it is dead.
    assert_eq!(
        http.post("/auth/logout").bearer(token).send().await.status,
        204
    );
    http.post("/auth/logout")
        .bearer(token)
        .send()
        .await
        .problem(401);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn raw_token_is_never_stored(pool: PgPool) {
    let http = setup(&pool).await;
    let app = login_app(&http, "anna@school.edu", PASSWORD).await;
    let web = login_web(&http, "anna@school.edu", PASSWORD).await;

    for token in [&app, &web] {
        let expected = hex(&Sha256::digest(token.as_bytes()));
        let stored = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM identity.sessions WHERE encode(token_hash, 'hex') = $1"#,
            expected
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(stored, 1, "the SHA-256 of the token is stored");
        let leaked = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM identity.sessions s WHERE s::text LIKE '%' || $1 || '%'"#,
            token
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(leaked, 0, "the raw token appears nowhere in the table");
    }
    let kinds =
        sqlx::query_scalar!(r#"SELECT kind AS "kind!" FROM identity.sessions ORDER BY kind"#)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(kinds, ["app", "web"]);
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn wrong_password_and_unknown_email_return_identical_401(pool: PgPool) {
    let http = setup(&pool).await;
    seed_user(&pool, "gone@school.edu", Role::Student).await;
    sqlx::query!("UPDATE identity.users SET status = 'disabled' WHERE email = 'gone@school.edu'")
        .execute(&pool)
        .await
        .unwrap();

    let mut responses = Vec::new();
    for (email, password) in [
        ("anna@school.edu", "wrong password entirely"),
        ("nobody@school.edu", PASSWORD),
        ("not-an-email", PASSWORD),
        ("gone@school.edu", PASSWORD), // disabled, with the right password
    ] {
        let response = http
            .post("/auth/login")
            .json(&login_body(email, password, "app"))
            .send()
            .await;
        response.problem(401);
        responses.push((
            response.header("www-authenticate").to_owned(),
            response.body,
        ));
    }

    assert!(
        responses.windows(2).all(|pair| pair[0] == pair[1]),
        "every failure must look the same: {responses:?}"
    );
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn login_rejects_malformed_requests_with_field_errors(pool: PgPool) {
    let http = setup(&pool).await;

    let empty = http
        .post("/auth/login")
        .json(&login_body("", PASSWORD, "app"))
        .send()
        .await
        .problem(422);
    assert_eq!(empty.errors[0].field, "email");

    let unknown_client = http
        .post("/auth/login")
        .json(&login_body("anna@school.edu", PASSWORD, "toaster"))
        .send()
        .await;
    unknown_client.problem(422);

    let not_json = http.post("/auth/login").send().await;
    assert_eq!(not_json.problem(415).status, 415);
}

// --- rate limiting -----------------------------------------------------------------------------

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn login_is_rate_limited_per_ip(pool: PgPool) {
    let http = setup(&pool).await;
    // Malformed bodies are rejected cheaply but still count against the client's quota.
    let mut statuses = Vec::new();
    for _ in 0..60 {
        statuses.push(
            http.post("/auth/login")
                .ip("203.0.113.7")
                .json(&json!({}))
                .send()
                .await
                .status,
        );
    }

    assert_eq!(statuses[0], 422);
    assert_eq!(statuses[59], 429);
    let limited = http
        .post("/auth/login")
        .ip("203.0.113.7")
        .json(&json!({}))
        .send()
        .await;
    limited.problem(429);

    let other_client = http
        .post("/auth/login")
        .ip("203.0.113.8")
        .json(&json!({}))
        .send()
        .await;
    assert_eq!(other_client.status, 422, "another IP has its own quota");
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn login_is_rate_limited_per_account(pool: PgPool) {
    let http = setup(&pool).await;
    seed_user(&pool, "other@school.edu", Role::Student).await;

    // Ten failures from ten different addresses: the per-IP limit never triggers.
    for i in 0..10 {
        let response = http
            .post("/auth/login")
            .ip(&format!("198.51.100.{i}"))
            .json(&login_body(
                "anna@school.edu",
                "wrong password entirely",
                "app",
            ))
            .send()
            .await;
        response.problem(401);
    }

    // Even the right password is refused now, before any hashing happens.
    let blocked = http
        .post("/auth/login")
        .ip("198.51.100.99")
        .json(&login_body("ANNA@school.edu", PASSWORD, "app"))
        .send()
        .await;
    blocked.problem(429);
    assert!(blocked.header("retry-after").parse::<u64>().unwrap() > 0);

    let other = http
        .post("/auth/login")
        .ip("198.51.100.99")
        .json(&login_body("other@school.edu", PASSWORD, "app"))
        .send()
        .await;
    assert_eq!(other.status, 200, "other accounts are unaffected");
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn unknown_emails_are_throttled_too(pool: PgPool) {
    let http = setup(&pool).await;
    for i in 0..10 {
        http.post("/auth/login")
            .ip(&format!("198.51.100.{i}"))
            .json(&login_body("ghost@school.edu", PASSWORD, "app"))
            .send()
            .await
            .problem(401);
    }
    let blocked = http
        .post("/auth/login")
        .ip("198.51.100.99")
        .json(&login_body("ghost@school.edu", PASSWORD, "app"))
        .send()
        .await;
    blocked.problem(429);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_successful_login_resets_the_account_counter(pool: PgPool) {
    let http = setup(&pool).await;
    for round in 0..3 {
        for i in 0..9 {
            http.post("/auth/login")
                .ip(&format!("198.51.100.{}", round * 10 + i))
                .json(&login_body(
                    "anna@school.edu",
                    "wrong password entirely",
                    "app",
                ))
                .send()
                .await
                .problem(401);
        }
        let ok = http
            .post("/auth/login")
            .ip("198.51.100.200")
            .json(&login_body("anna@school.edu", PASSWORD, "app"))
            .send()
            .await;
        assert_eq!(ok.status, 200, "round {round}");
    }
}

// --- sessions ----------------------------------------------------------------------------------

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn expired_session_is_rejected(pool: PgPool) {
    let http = setup(&pool).await;
    let token = login_app(&http, "anna@school.edu", PASSWORD).await;
    sqlx::query!("UPDATE identity.sessions SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();

    http.post("/auth/logout")
        .bearer(&token)
        .send()
        .await
        .problem(401);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn disabled_users_session_is_rejected(pool: PgPool) {
    let http = setup(&pool).await;
    let token = login_app(&http, "anna@school.edu", PASSWORD).await;
    sqlx::query!("UPDATE identity.users SET status = 'disabled'")
        .execute(&pool)
        .await
        .unwrap();

    http.post("/auth/logout")
        .bearer(&token)
        .send()
        .await
        .problem(401);
    http.post("/auth/login")
        .json(&login_body("anna@school.edu", PASSWORD, "app"))
        .send()
        .await
        .problem(401);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn unauthenticated_and_malformed_credentials_are_401(pool: PgPool) {
    let http = setup(&pool).await;
    let token = login_app(&http, "anna@school.edu", PASSWORD).await;

    let none = http.post("/auth/logout").send().await;
    none.problem(401);
    assert_eq!(none.header("www-authenticate"), "Bearer");
    http.post("/auth/logout")
        .bearer("short")
        .send()
        .await
        .problem(401);
    http.post("/auth/logout")
        .bearer(&"A".repeat(43))
        .send()
        .await
        .problem(401);
    http.post("/auth/logout")
        .cookie("not-a-token")
        .send()
        .await
        .problem(401);
    // A valid token in the wrong scheme is not accepted either.
    http.post("/auth/logout")
        .header("authorization", &format!("Basic {token}"))
        .send()
        .await
        .problem(401);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn logout_deletes_the_session_and_clears_the_cookie(pool: PgPool) {
    let http = setup(&pool).await;
    let web = login_web(&http, "anna@school.edu", PASSWORD).await;
    let other = login_app(&http, "anna@school.edu", PASSWORD).await;

    let response = http
        .post("/auth/logout")
        .cookie(&web)
        .origin(ORIGIN)
        .send()
        .await;

    assert_eq!(response.status, 204);
    let cleared = response.header("set-cookie");
    assert!(
        cleared.starts_with("learnly_session=;") && cleared.contains("Max-Age=0"),
        "{cleared}"
    );
    http.post("/auth/logout")
        .cookie(&web)
        .origin(ORIGIN)
        .send()
        .await
        .problem(401);
    let remaining = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM identity.sessions"#)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(remaining, 1, "only the other session is left");
    assert_eq!(
        http.post("/auth/logout").bearer(&other).send().await.status,
        204
    );
}

// --- CSRF --------------------------------------------------------------------------------------

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn cookie_post_without_matching_origin_is_403(pool: PgPool) {
    let http = setup(&pool).await;
    let web = login_web(&http, "anna@school.edu", PASSWORD).await;

    http.post("/auth/logout")
        .cookie(&web)
        .send()
        .await
        .problem(403);
    http.post("/auth/logout")
        .cookie(&web)
        .origin("https://evil.test")
        .send()
        .await
        .problem(403);
    http.post("/auth/logout")
        .cookie(&web)
        .origin("null")
        .send()
        .await
        .problem(403);

    // None of the refused requests ended the session.
    assert_eq!(
        http.post("/auth/logout")
            .cookie(&web)
            .origin(ORIGIN)
            .send()
            .await
            .status,
        204
    );
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn bearer_post_without_origin_is_allowed(pool: PgPool) {
    let http = setup(&pool).await;
    let token = login_app(&http, "anna@school.edu", PASSWORD).await;

    assert_eq!(
        http.post("/auth/logout").bearer(&token).send().await.status,
        204
    );
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn web_login_requires_matching_origin(pool: PgPool) {
    let http = setup(&pool).await;
    let body = login_body("anna@school.edu", PASSWORD, "web");

    http.post("/auth/login")
        .json(&body)
        .send()
        .await
        .problem(403);
    http.post("/auth/login")
        .origin("https://evil.test")
        .json(&body)
        .send()
        .await
        .problem(403);
    assert_eq!(
        http.post("/auth/login")
            .origin(ORIGIN)
            .json(&body)
            .send()
            .await
            .status,
        200
    );
    // Bearer clients are not browsers: app login needs no Origin.
    assert_eq!(
        http.post("/auth/login")
            .json(&login_body("anna@school.edu", PASSWORD, "app"))
            .send()
            .await
            .status,
        200
    );
}

// --- /me ---------------------------------------------------------------------------------------

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn me_returns_the_current_user(pool: PgPool) {
    let http = setup(&pool).await;
    let token = login_app(&http, "anna@school.edu", PASSWORD).await;

    let response = http.get("/me").bearer(&token).send().await;

    assert_eq!(response.status, 200);
    let me = response.json();
    assert_eq!(me["email"], "anna@school.edu");
    assert_eq!(me["display_name"], "Test User");
    assert_eq!(me["role"], "student");
    assert_eq!(me["status"], "active");
    assert_eq!(me["must_change_password"], false);
    assert!(me.get("password_hash").is_none());
    http.get("/me").send().await.problem(401);
}

// --- forced password change --------------------------------------------------------------------

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn pending_password_change_allows_only_me_logout_and_password(pool: PgPool) {
    let temporary = seed_pending_user(&pool, "newbie@school.edu", Role::Teacher).await;
    let http = Http::new(&spawn(pool.clone()).await);
    let token = login_app(&http, "newbie@school.edu", &temporary).await;
    let id = uuid::Uuid::now_v7();

    // Everything else is refused with 403 ...
    http.get("/me/sessions")
        .bearer(&token)
        .send()
        .await
        .problem(403);
    http.delete(&format!("/me/sessions/{id}"))
        .bearer(&token)
        .send()
        .await
        .problem(403);

    // ... except these three.
    let me = http.get("/me").bearer(&token).send().await;
    assert_eq!(me.status, 200);
    assert_eq!(me.json()["must_change_password"], true);
    let changed = http
        .post("/me/password")
        .bearer(&token)
        .json(&json!({"current_password": temporary, "new_password": PASSWORD}))
        .send()
        .await;
    assert_eq!(
        changed.status,
        204,
        "{}",
        String::from_utf8_lossy(&changed.body)
    );

    assert_eq!(
        http.get("/me").bearer(&token).send().await.json()["must_change_password"],
        false
    );
    assert_eq!(
        http.get("/me/sessions").bearer(&token).send().await.status,
        200
    );
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn pending_user_can_log_out(pool: PgPool) {
    let temporary = seed_pending_user(&pool, "newbie@school.edu", Role::Student).await;
    let http = Http::new(&spawn(pool.clone()).await);
    let token = login_app(&http, "newbie@school.edu", &temporary).await;

    assert_eq!(
        http.post("/auth/logout").bearer(&token).send().await.status,
        204
    );
}

// --- change password ---------------------------------------------------------------------------

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn changing_password_revokes_other_sessions_only(pool: PgPool) {
    let http = setup(&pool).await;
    let current = login_app(&http, "anna@school.edu", PASSWORD).await;
    let phone = login_app(&http, "anna@school.edu", PASSWORD).await;
    let browser = login_web(&http, "anna@school.edu", PASSWORD).await;
    seed_user(&pool, "bob@school.edu", Role::Student).await;
    let bobs = login_app(&http, "bob@school.edu", PASSWORD).await;
    let new_password = "a brand new passphrase, with commas";

    let response = http
        .post("/me/password")
        .bearer(&current)
        .json(&json!({"current_password": PASSWORD, "new_password": new_password}))
        .send()
        .await;

    assert_eq!(
        response.status,
        204,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert_eq!(
        http.get("/me").bearer(&current).send().await.status,
        200,
        "the current session stays"
    );
    http.get("/me").bearer(&phone).send().await.problem(401);
    http.get("/me").cookie(&browser).send().await.problem(401);
    assert_eq!(
        http.get("/me").bearer(&bobs).send().await.status,
        200,
        "other users are untouched"
    );
    // The new password works, the old one does not.
    login_app(&http, "anna@school.edu", new_password).await;
    http.post("/auth/login")
        .ip("192.0.2.1")
        .json(&login_body("anna@school.edu", PASSWORD, "app"))
        .send()
        .await
        .problem(401);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn weak_new_password_returns_422_with_errors(pool: PgPool) {
    let http = setup(&pool).await;
    let token = login_app(&http, "anna@school.edu", PASSWORD).await;
    let change = |new: &str| {
        http.post("/me/password")
            .bearer(&token)
            .json(&json!({"current_password": PASSWORD, "new_password": new}))
            .send()
    };

    let common = change("password1234").await.problem(422);
    assert!(!common.errors.is_empty(), "feedback is returned");
    assert!(common.errors.iter().all(|e| e.field == "new_password"));
    assert!(
        common
            .errors
            .iter()
            .all(|e| !e.message.contains("password1234"))
    );

    let short = change("Tr0ub4dor&3").await.problem(422);
    assert_eq!(short.errors[0].message, "must be at least 12 characters");

    let personal = change("annaschool").await.problem(422);
    assert_eq!(personal.errors[0].field, "new_password");

    let same = change(PASSWORD).await.problem(422);
    assert!(same.errors[0].message.contains("differ"), "{same:?}");

    let accepted = change("zażółć gęślą jaźń — wieczorem nad łąką").await;
    assert_eq!(accepted.status, 204);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn wrong_current_password_is_rejected_and_throttled(pool: PgPool) {
    let http = setup(&pool).await;
    let token = login_app(&http, "anna@school.edu", PASSWORD).await;
    let attempt = |current: &str| {
        http.post("/me/password")
            .bearer(&token)
            .json(&json!({"current_password": current, "new_password": "a long enough new passphrase"}))
            .send()
    };

    for _ in 0..10 {
        let response = attempt("not the password at all").await.problem(422);
        assert_eq!(response.errors[0].field, "current_password");
    }

    // A stolen session cannot be used to guess the password forever.
    attempt(PASSWORD).await.problem(429);
}

// --- sliding sessions --------------------------------------------------------------------------

async fn session_times(
    pool: &PgPool,
) -> (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) {
    let row = sqlx::query!(
        r#"SELECT last_seen_at AS "last_seen_at!", expires_at AS "expires_at!"
           FROM identity.sessions"#
    )
    .fetch_one(pool)
    .await
    .unwrap();
    (row.last_seen_at, row.expires_at)
}

async fn age_session(pool: &PgPool) {
    sqlx::query!(
        "UPDATE identity.sessions
         SET last_seen_at = now() - interval '2 hours', expires_at = now() + interval '1 day'"
    )
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn sliding_expiry_is_extended_at_most_once_per_hour(pool: PgPool) {
    for (client, days) in [("web", 30), ("app", 90)] {
        sqlx::query!("DELETE FROM identity.sessions")
            .execute(&pool)
            .await
            .unwrap();
        let http = setup_once(&pool).await;
        let token = if client == "web" {
            login_web(&http, "anna@school.edu", PASSWORD).await
        } else {
            login_app(&http, "anna@school.edu", PASSWORD).await
        };
        let get_me = || async {
            let request = http.get("/me");
            let request = if client == "web" {
                request.cookie(&token)
            } else {
                request.bearer(&token)
            };
            assert_eq!(request.send().await.status, 200);
        };

        // Seen two hours ago: the next request refreshes it.
        age_session(&pool).await;
        get_me().await;
        let (seen, expires) = session_times(&pool).await;
        let slid = (expires - chrono::Utc::now()).num_days();
        assert!(
            ((days - 1)..=days).contains(&slid),
            "{client}: expiry slid to {slid} days"
        );
        assert!(
            (chrono::Utc::now() - seen).num_seconds() < 60,
            "{client}: last_seen_at refreshed"
        );

        // Requests within the hour change nothing, not even last_seen_at.
        get_me().await;
        get_me().await;
        assert_eq!(session_times(&pool).await, (seen, expires), "{client}");

        // An hour later it may be refreshed again.
        age_session(&pool).await;
        get_me().await;
        assert_ne!(session_times(&pool).await, (seen, expires), "{client}");
    }
}

/// `setup` seeds the user once per test; the loop above reuses it.
async fn setup_once(pool: &PgPool) -> Http {
    let exists = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM identity.users"#)
        .fetch_one(pool)
        .await
        .unwrap();
    if exists == 0 {
        return setup(pool).await;
    }
    Http::new(&spawn(pool.clone()).await)
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn absolute_sessions_are_seen_but_never_extended(pool: PgPool) {
    let http = setup(&pool).await;
    let token = login_app(&http, "anna@school.edu", PASSWORD).await;
    sqlx::query!(
        "UPDATE identity.sessions
         SET kind = 'cli', last_seen_at = now() - interval '2 hours', expires_at = now() + interval '5 days'"
    )
    .execute(&pool)
    .await
    .unwrap();
    let (_, expires_before) = session_times(&pool).await;

    assert_eq!(http.get("/me").bearer(&token).send().await.status, 200);

    let (seen, expires) = session_times(&pool).await;
    assert!((chrono::Utc::now() - seen).num_seconds() < 60);
    assert_eq!(
        expires, expires_before,
        "a cli session has an absolute expiry"
    );
}

// --- own sessions ------------------------------------------------------------------------------

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn sessions_list_marks_the_current_one_and_pages(pool: PgPool) {
    let http = setup(&pool).await;
    seed_user(&pool, "bob@school.edu", Role::Student).await;
    login_app(&http, "bob@school.edu", PASSWORD).await;
    let first = login_app(&http, "anna@school.edu", PASSWORD).await;
    login_web(&http, "anna@school.edu", PASSWORD).await;
    let third = login_app(&http, "anna@school.edu", PASSWORD).await;

    let page1 = http.get("/me/sessions?limit=2").bearer(&third).send().await;
    assert_eq!(page1.status, 200);
    let body = page1.json();
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    let cursor = body["next_cursor"]
        .as_str()
        .expect("there is a next page")
        .to_owned();

    let page2 = http
        .get(&format!("/me/sessions?limit=2&cursor={cursor}"))
        .bearer(&third)
        .send()
        .await
        .json();
    let rest = page2["items"].as_array().unwrap();
    assert_eq!(rest.len(), 1);
    assert!(page2["next_cursor"].is_null());

    let all: Vec<&serde_json::Value> = items.iter().chain(rest).collect();
    let kinds: Vec<&str> = all.iter().map(|s| s["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["app", "web", "app"], "oldest first, only Anna's");
    let current: Vec<bool> = all
        .iter()
        .map(|s| s["current"].as_bool().unwrap())
        .collect();
    assert_eq!(current, [false, false, true]);
    assert!(
        all.iter()
            .all(|s| s.get("token").is_none() && s.get("token_hash").is_none())
    );

    // The same list from the first session flags a different one as current.
    let from_first = http.get("/me/sessions").bearer(&first).send().await.json();
    assert_eq!(from_first["items"][0]["current"], true);
    assert_eq!(from_first["items"].as_array().unwrap().len(), 3);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn sessions_list_hides_expired_ones_and_rejects_bad_cursors(pool: PgPool) {
    let http = setup(&pool).await;
    let stale = login_app(&http, "anna@school.edu", PASSWORD).await;
    let token = login_app(&http, "anna@school.edu", PASSWORD).await;
    let hash = hex(&Sha256::digest(stale.as_bytes()));
    sqlx::query!(
        "UPDATE identity.sessions SET expires_at = now() - interval '1 day'
         WHERE encode(token_hash, 'hex') = $1",
        hash
    )
    .execute(&pool)
    .await
    .unwrap();

    let list = http.get("/me/sessions").bearer(&token).send().await.json();
    assert_eq!(list["items"].as_array().unwrap().len(), 1);

    let bad = http
        .get("/me/sessions?cursor=junk")
        .bearer(&token)
        .send()
        .await
        .problem(422);
    assert_eq!(bad.errors[0].field, "cursor");
    http.get("/me/sessions?limit=0")
        .bearer(&token)
        .send()
        .await
        .problem(422);
    http.get("/me/sessions?limit=101")
        .bearer(&token)
        .send()
        .await
        .problem(422);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_user_can_revoke_their_own_sessions_only(pool: PgPool) {
    let http = setup(&pool).await;
    seed_user(&pool, "bob@school.edu", Role::Student).await;
    let bobs = login_app(&http, "bob@school.edu", PASSWORD).await;
    let phone = login_app(&http, "anna@school.edu", PASSWORD).await;
    let laptop = login_app(&http, "anna@school.edu", PASSWORD).await;
    let listed = http.get("/me/sessions").bearer(&laptop).send().await.json();
    let phone_id = listed["items"][0]["id"].as_str().unwrap().to_owned();
    let bobs_list = http.get("/me/sessions").bearer(&bobs).send().await.json();
    let bobs_id = bobs_list["items"][0]["id"].as_str().unwrap().to_owned();

    // Someone else's session and an unknown one both look missing.
    http.delete(&format!("/me/sessions/{bobs_id}"))
        .bearer(&laptop)
        .send()
        .await
        .problem(404);
    http.delete(&format!("/me/sessions/{}", uuid::Uuid::now_v7()))
        .bearer(&laptop)
        .send()
        .await
        .problem(404);
    assert_eq!(http.get("/me").bearer(&bobs).send().await.status, 200);

    let revoked = http
        .delete(&format!("/me/sessions/{phone_id}"))
        .bearer(&laptop)
        .send()
        .await;
    assert_eq!(revoked.status, 204);
    http.get("/me").bearer(&phone).send().await.problem(401);
    assert_eq!(http.get("/me").bearer(&laptop).send().await.status, 200);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn revoking_with_a_cookie_needs_the_origin(pool: PgPool) {
    let http = setup(&pool).await;
    let web = login_web(&http, "anna@school.edu", PASSWORD).await;
    let id = uuid::Uuid::now_v7();

    http.delete(&format!("/me/sessions/{id}"))
        .cookie(&web)
        .send()
        .await
        .problem(403);
    http.delete(&format!("/me/sessions/{id}"))
        .cookie(&web)
        .origin(ORIGIN)
        .send()
        .await
        .problem(404);
}
