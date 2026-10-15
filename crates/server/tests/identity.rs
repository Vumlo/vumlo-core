mod common;

use common::{Http, ORIGIN, PASSWORD, login_app, login_web, seed_user, spawn};
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
