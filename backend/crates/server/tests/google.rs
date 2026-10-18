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
