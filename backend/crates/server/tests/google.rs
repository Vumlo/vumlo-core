//! ADR-0047: sign-in with Google, against a fake issuer.

mod common;

use common::google::{CLIENT_ID, CLIENT_SECRET, FakeIssuer, Person, REDIRECT_URI};
use common::{Http, ORIGIN, PASSWORD, Resp, db_test, seed_user, spawn_app, test_settings};
use server::identity::Role;
use sqlx::PgPool;
use vumlo_config::{GoogleSettings, Secret, Settings};

fn settings(issuer_url: &str) -> Settings {
    let mut settings = test_settings();
    settings.auth.allowed_domains = vec!["lms.test".to_owned()];
    settings.auth.google = Some(GoogleSettings {
        client_id: CLIENT_ID.to_owned(),
        client_secret: Secret::new(CLIENT_SECRET),
        redirect_uri: REDIRECT_URI.to_owned(),
        issuer: issuer_url.to_owned(),
    });
    settings
}

async fn setup(pool: &PgPool) -> (Http, FakeIssuer) {
    let issuer = FakeIssuer::start().await;
    let (base, _) = spawn_app(pool.clone(), settings(&issuer.base)).await;
    (Http::new(&base), issuer)
}

/// The value of the cookie `name` that `response` sets.
fn set_cookie(response: &Resp, name: &str) -> Option<String> {
    response
        .headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|line| line.strip_prefix(&format!("{name}="))?.split(';').next())
        .map(str::to_owned)
}

/// Starts a sign-in; returns the authorization URL and the browser's binding cookie.
async fn start(http: &Http, hint: Option<&str>) -> (String, String) {
    let body = match hint {
        Some(hint) => serde_json::json!({"login_hint": hint}),
        None => serde_json::json!({}),
    };
    let response = http.post("/auth/google/start").json(&body).send().await;
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert_eq!(response.header("cache-control"), "no-store");
    let cookie_line = response
        .headers
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .find(|line| line.starts_with("vumlo_oauth="))
        .expect("the binding cookie");
    for attribute in [
        "HttpOnly",
        "SameSite=Lax",
        "Path=/api/v1/auth/google",
        "Max-Age=600",
    ] {
        assert!(
            cookie_line.contains(attribute),
            "{attribute}: {cookie_line}"
        );
    }
    let binding = set_cookie(&response, "vumlo_oauth").unwrap();
    (
        response.json()["authorization_url"]
            .as_str()
            .unwrap()
            .to_owned(),
        binding,
    )
}

async fn callback(http: &Http, code: &str, state: &str, binding: Option<&str>) -> Resp {
    let mut request = http
        .post("/auth/google/callback")
        .origin(ORIGIN)
        .json(&serde_json::json!({"code": code, "state": state}));
    if let Some(binding) = binding {
        request = request.header("cookie", &format!("vumlo_oauth={binding}"));
    }
    request.send().await
}

/// A complete sign-in of `person`.
async fn sign_in(http: &Http, issuer: &FakeIssuer, person: Person) -> Resp {
    let (url, binding) = start(http, None).await;
    let (code, state) = issuer.approve(&url, person);
    callback(http, &code, &state, Some(&binding)).await
}

#[tokio::test]
async fn a_first_sign_in_creates_a_verified_student_and_opens_a_web_session() {
    db_test(async |pool| {
        let (http, issuer) = setup(&pool).await;
        let (url, binding) = start(&http, Some("maya@lms.test")).await;
        assert!(url.contains("login_hint=maya%40lms.test"), "{url}");
        let (code, state) = issuer.approve(&url, Person::new("sub-1", "Maya@LMS.test"));

        let response = callback(&http, &code, &state, Some(&binding)).await;

        assert_eq!(
            response.status,
            200,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        assert_eq!(response.header("cache-control"), "no-store");
        let user = response.json();
        assert_eq!(user["email"], "maya@lms.test");
        assert_eq!(
            (user["name"].as_str(), user["surname"].as_str()),
            (Some("Maya"), Some("Okafor"))
        );
        assert_eq!(user["role"], "student");
        assert_eq!(user["must_change_password"], false);
        let session = set_cookie(&response, "vumlo_session").expect("a session cookie");
        assert_eq!(
            set_cookie(&response, "vumlo_oauth").as_deref(),
            Some(""),
            "the binding is cleared"
        );
        assert_eq!(http.get("/me").cookie(&session).send().await.status, 200);

        let account = sqlx::query!(
            r#"SELECT u.email_verified_at IS NOT NULL AS "verified!",
                      (SELECT count(*) FROM identity.identities i
                        WHERE i.user_id = u.id AND i.provider = 'password') AS "passwords!",
                      (SELECT subject FROM identity.identities i
                        WHERE i.user_id = u.id AND i.provider = 'google') AS google
               FROM identity.users u WHERE u.email = 'maya@lms.test'"#
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(account.verified);
        assert_eq!(account.passwords, 0, "no password was ever set");
        assert_eq!(account.google.as_deref(), Some("sub-1"));

        // The same Google account next time is the same user.
        let again = sign_in(&http, &issuer, Person::new("sub-1", "maya@lms.test")).await;
        assert_eq!(again.json()["id"], user["id"]);
        let users = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM identity.users"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(users, 1);
    })
    .await;
}

#[tokio::test]
async fn the_names_fall_back_to_the_address_and_the_longest_are_cut() {
    db_test(async |pool| {
        let (http, issuer) = setup(&pool).await;
        let mut person = Person::new("sub-2", "omar.haddad@lms.test");
        person.given_name = None;
        person.family_name = Some("x".repeat(300));

        let response = sign_in(&http, &issuer, person).await;

        let user = response.json();
        assert_eq!(user["name"], "omar.haddad");
        assert_eq!(user["surname"].as_str().unwrap().chars().count(), 100);
    })
    .await;
}

#[tokio::test]
async fn without_the_google_section_both_endpoints_are_off() {
    db_test(async |pool| {
        let (base, _) = spawn_app(pool, test_settings()).await;
        let http = Http::new(&base);

        http.post("/auth/google/start")
            .json(&serde_json::json!({}))
            .send()
            .await
            .problem(404);
        callback(&http, "code", "state", None).await.problem(404);
    })
    .await;
}

#[tokio::test]
async fn an_attempt_works_once_in_the_browser_that_started_it() {
    db_test(async |pool| {
        let (http, issuer) = setup(&pool).await;
        let (url, binding) = start(&http, None).await;
        let (_, other_binding) = start(&http, None).await;
        let (code, state) = issuer.approve(&url, Person::new("sub-3", "maya@lms.test"));

        // Another browser, or none at all, cannot finish it.
        callback(&http, &code, &state, Some(&other_binding))
            .await
            .problem(401);
        callback(&http, &code, &state, None).await.problem(401);
        callback(&http, &code, "an unknown state", Some(&binding))
            .await
            .problem(401);
        // The wrong attempts did not use it up; the right browser can.
        assert_eq!(
            callback(&http, &code, &state, Some(&binding)).await.status,
            200
        );
        // And only once.
        callback(&http, &code, &state, Some(&binding))
            .await
            .problem(401);
    })
    .await;
}

#[tokio::test]
async fn an_expired_attempt_and_a_foreign_origin_are_refused() {
    db_test(async |pool| {
        let (http, issuer) = setup(&pool).await;
        let (url, binding) = start(&http, None).await;
        let (code, state) = issuer.approve(&url, Person::new("sub-4", "maya@lms.test"));

        let foreign = http
            .post("/auth/google/callback")
            .origin("https://evil.test")
            .header("cookie", &format!("vumlo_oauth={binding}"))
            .json(&serde_json::json!({"code": code, "state": state}))
            .send()
            .await;
        foreign.problem(403);

        sqlx::query!("UPDATE identity.oauth_states SET created_at = now() - interval '11 minutes', expires_at = now() - interval '1 minute'")
            .execute(&pool)
            .await
            .unwrap();
        callback(&http, &code, &state, Some(&binding)).await.problem(401);
        // Starting another attempt forgets the expired one.
        start(&http, None).await;
        let left = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM identity.oauth_states"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 1);
    })
    .await;
}

#[tokio::test]
async fn an_unverified_google_email_a_foreign_domain_and_a_wrong_nonce_do_not_sign_in() {
    db_test(async |pool| {
        let (http, issuer) = setup(&pool).await;
        let mut unverified = Person::new("sub-5", "maya@lms.test");
        unverified.email_verified = false;
        sign_in(&http, &issuer, unverified).await.problem(403);
        sign_in(&http, &issuer, Person::new("sub-6", "maya@gmail.test"))
            .await
            .problem(403);
        let mut forged = Person::new("sub-7", "maya@lms.test");
        forged.wrong_nonce = true;
        sign_in(&http, &issuer, forged).await.problem(401);

        let users = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM identity.users"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(users, 0);
    })
    .await;
}

#[tokio::test]
async fn an_existing_account_with_that_address_is_linked_and_keeps_its_password() {
    db_test(async |pool| {
        let (http, issuer) = setup(&pool).await;
        seed_user(&pool, "maya@lms.test", Role::Teacher).await;

        let response = sign_in(&http, &issuer, Person::new("sub-8", "maya@lms.test")).await;

        assert_eq!(response.status, 200);
        assert_eq!(response.json()["role"], "teacher", "the account keeps its role");
        let login = http
            .post("/auth/login")
            .json(&serde_json::json!({"email": "maya@lms.test", "password": PASSWORD, "client": "app"}))
            .send()
            .await;
        assert_eq!(login.status, 200, "the password still works");
        let identities = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM identity.identities"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(identities, 2);
    })
    .await;
}

#[tokio::test]
async fn google_takes_the_password_of_an_account_registered_with_an_address_nobody_verified() {
    db_test(async |pool| {
        let (http, issuer) = setup(&pool).await;
        // A stranger pre-registered the address with a password of their own.
        let mut settings = settings(&issuer.base);
        let mailbox = common::Mailbox::new();
        settings.mail = mailbox.settings().mail;
        let (base, state) = spawn_app(pool.clone(), settings).await;
        let registering = Http::new(&base);
        registering
            .post("/auth/register")
            .json(&serde_json::json!({"name": "Stranger", "surname": "S", "email": "maya@lms.test", "password": PASSWORD}))
            .send()
            .await;
        server::events::process_once(&state.db, &state.event_handlers()).await.unwrap();
        assert_eq!(mailbox.messages().len(), 1);

        let response = sign_in(&http, &issuer, Person::new("sub-9", "maya@lms.test")).await;

        assert_eq!(response.status, 200);
        let login = registering
            .post("/auth/login")
            .json(&serde_json::json!({"email": "maya@lms.test", "password": PASSWORD, "client": "app"}))
            .send()
            .await;
        assert_eq!(login.status, 401, "the stranger's password is gone");
        let account = sqlx::query!(
            r#"SELECT email_verified_at IS NOT NULL AS "verified!",
                      (SELECT count(*) FROM identity.email_tokens WHERE used_at IS NULL) AS "open_tokens!"
               FROM identity.users WHERE email = 'maya@lms.test'"#
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(account.verified);
        assert_eq!(account.open_tokens, 0, "the pending verification link is void");
    })
    .await;
}

#[tokio::test]
async fn a_disabled_account_cannot_sign_in_with_google() {
    db_test(async |pool| {
        let (http, issuer) = setup(&pool).await;
        sign_in(&http, &issuer, Person::new("sub-10", "maya@lms.test")).await;
        sqlx::query!("UPDATE identity.users SET status = 'disabled'")
            .execute(&pool)
            .await
            .unwrap();

        let response = sign_in(&http, &issuer, Person::new("sub-10", "maya@lms.test")).await;

        response.problem(403);
        assert!(set_cookie(&response, "vumlo_session").is_none());
    })
    .await;
}

#[tokio::test]
async fn an_unreachable_google_is_a_503_that_can_be_tried_again() {
    db_test(async |pool| {
        // Port 1 is closed, so discovery fails.
        let (base, _) = spawn_app(pool, settings("http://127.0.0.1:1")).await;
        let http = Http::new(&base);

        let response = http
            .post("/auth/google/start")
            .json(&serde_json::json!({}))
            .send()
            .await;

        response.problem(503);
        assert!(response.headers.contains_key("retry-after"));
    })
    .await;
}

// --- a native app's sign-in (ADR-0062) -----------------------------------------------------------

const VERIFIER: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn challenge_of(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(verifier.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Starts as an app does: no cookie comes back, and the attempt is tied to the challenge.
async fn start_as_app(http: &Http, verifier: &str) -> String {
    let response = http
        .post("/auth/google/start")
        .json(&serde_json::json!({"app_challenge": challenge_of(verifier)}))
        .send()
        .await;
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert_eq!(response.header("cache-control"), "no-store");
    assert!(
        response.headers.get("set-cookie").is_none(),
        "the app has no browser to tie it to"
    );
    response.json()["authorization_url"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn exchange(http: &Http, code: &str, verifier: &str) -> Resp {
    http.post("/auth/app/exchange")
        .json(&serde_json::json!({"code": code, "verifier": verifier}))
        .send()
        .await
}

/// The handoff of a complete app sign-in of `person`: the browser finishes it without any cookie.
async fn handoff_of(http: &Http, issuer: &FakeIssuer, person: Person, verifier: &str) -> String {
    let url = start_as_app(http, verifier).await;
    let (code, state) = issuer.approve(&url, person);
    let response = callback(http, &code, &state, None).await;
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert_eq!(response.header("cache-control"), "no-store");
    assert!(
        response.headers.get("set-cookie").is_none(),
        "no web session for an app"
    );
    let body = response.json();
    assert!(
        body.get("email").is_none(),
        "no user and no token reach the browser: {body}"
    );
    body["handoff"].as_str().expect("a handoff").to_owned()
}

#[tokio::test]
async fn an_app_signs_in_with_google_through_a_handoff_that_only_it_can_redeem() {
    db_test(async |pool| {
        let (http, issuer) = setup(&pool).await;
        let handoff = handoff_of(&http, &issuer, Person::new("sub-app", "maya@lms.test"), VERIFIER).await;

        let response = exchange(&http, &handoff, VERIFIER).await;
        assert_eq!(response.status, 200, "{}", String::from_utf8_lossy(&response.body));
        assert_eq!(response.header("cache-control"), "no-store");
        let body = response.json();
        assert_eq!(body["user"]["email"], "maya@lms.test");
        assert_eq!(body["user"]["role"], "student");
        let token = body["token"].as_str().unwrap();
        assert_eq!(http.get("/me").bearer(token).send().await.status, 200);

        let kind = sqlx::query_scalar!(
            r#"SELECT s.kind AS "kind!" FROM identity.sessions s JOIN identity.users u ON u.id = s.user_id
               WHERE u.email = 'maya@lms.test'"#
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(kind, "app");
        let verified = sqlx::query_scalar!(
            r#"SELECT email_verified_at IS NOT NULL AS "v!" FROM identity.users WHERE email = 'maya@lms.test'"#
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(verified, "the same account rules as the web sign-in");
    })
    .await;
}

#[tokio::test]
async fn a_handoff_is_used_once_and_a_wrong_secret_spends_it() {
    db_test(async |pool| {
        let (http, issuer) = setup(&pool).await;
        let handoff = handoff_of(
            &http,
            &issuer,
            Person::new("sub-1", "maya@lms.test"),
            VERIFIER,
        )
        .await;
        assert_eq!(
            exchange(&http, &handoff, &"f".repeat(64)).await.status,
            401,
            "another secret"
        );
        assert_eq!(
            exchange(&http, &handoff, VERIFIER).await.status,
            401,
            "and the code is gone"
        );

        let handoff = handoff_of(
            &http,
            &issuer,
            Person::new("sub-1", "maya@lms.test"),
            VERIFIER,
        )
        .await;
        assert_eq!(exchange(&http, &handoff, VERIFIER).await.status, 200);
        assert_eq!(
            exchange(&http, &handoff, VERIFIER).await.status,
            401,
            "used once"
        );
        assert_eq!(exchange(&http, "no-such-code", VERIFIER).await.status, 401);
    })
    .await;
}

#[tokio::test]
async fn a_handoff_expires_after_a_minute() {
    db_test(async |pool| {
        let (http, issuer) = setup(&pool).await;
        let handoff = handoff_of(&http, &issuer, Person::new("sub-1", "maya@lms.test"), VERIFIER).await;
        sqlx::query!(
            "UPDATE identity.app_handoffs SET created_at = now() - interval '2 minutes', expires_at = now() - interval '1 minute'"
        )
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(exchange(&http, &handoff, VERIFIER).await.status, 401);
    })
    .await;
}

#[tokio::test]
async fn an_apps_attempt_is_used_once_and_does_not_open_a_browser_session() {
    db_test(async |pool| {
        let (http, issuer) = setup(&pool).await;
        let url = start_as_app(&http, VERIFIER).await;
        let (code, state) = issuer.approve(&url, Person::new("sub-1", "maya@lms.test"));
        assert_eq!(callback(&http, &code, &state, None).await.status, 200);
        assert_eq!(
            callback(&http, &code, &state, None).await.status,
            401,
            "once"
        );
        // A browser's attempt still needs its cookie; an app's needs none.
        let (other_url, binding) = start(&http, None).await;
        let (other_code, other_state) =
            issuer.approve(&other_url, Person::new("sub-2", "other@lms.test"));
        assert_eq!(
            callback(&http, &other_code, &other_state, None)
                .await
                .status,
            401,
            "a browser's attempt needs its cookie"
        );
        let finished = callback(&http, &other_code, &other_state, Some(&binding)).await;
        assert_eq!(
            finished.status, 200,
            "the refusal did not spend it, as before"
        );
        assert!(
            set_cookie(&finished, "vumlo_session").is_some(),
            "a browser's attempt still opens a web session"
        );
    })
    .await;
}

#[tokio::test]
async fn an_app_challenge_that_is_not_a_hash_is_refused_before_google_is_asked() {
    db_test(async |pool| {
        let (http, _issuer) = setup(&pool).await;
        for bad in [
            "",
            "abc",
            &"A".repeat(64),
            &"g".repeat(64),
            &"a".repeat(63),
            &"a".repeat(65),
        ] {
            let response = http
                .post("/auth/google/start")
                .json(&serde_json::json!({"app_challenge": bad}))
                .send()
                .await;
            assert_eq!(response.status, 422, "{bad:?}");
        }
    })
    .await;
}

#[tokio::test]
async fn an_apps_sign_in_has_the_same_limits_as_the_browser_s() {
    db_test(async |pool| {
        let (http, issuer) = setup(&pool).await;
        // A foreign origin cannot finish it, a foreign domain and an unverified address do not sign in.
        let url = start_as_app(&http, VERIFIER).await;
        let (code, state) = issuer.approve(&url, Person::new("sub-1", "maya@lms.test"));
        let foreign = http
            .post("/auth/google/callback")
            .origin("https://evil.test")
            .json(&serde_json::json!({"code": code, "state": state}))
            .send()
            .await;
        assert_eq!(foreign.status, 403);

        let url = start_as_app(&http, VERIFIER).await;
        let (code, state) = issuer.approve(&url, Person::new("sub-3", "eve@elsewhere.test"));
        assert_eq!(
            callback(&http, &code, &state, None).await.status,
            403,
            "a foreign domain"
        );
        let count = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM identity.app_handoffs"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "no handoff for either");
    })
    .await;
}

#[tokio::test]
async fn an_account_disabled_after_the_sign_in_gets_no_session_from_the_handoff() {
    db_test(async |pool| {
        let (http, issuer) = setup(&pool).await;
        let handoff = handoff_of(
            &http,
            &issuer,
            Person::new("sub-1", "maya@lms.test"),
            VERIFIER,
        )
        .await;
        sqlx::query!("UPDATE identity.users SET status = 'disabled' WHERE email = 'maya@lms.test'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(exchange(&http, &handoff, VERIFIER).await.status, 403);
        let sessions = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM identity.sessions"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(sessions, 0);
    })
    .await;
}
