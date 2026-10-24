//! Trusted frontends on another origin (ADR-0041): CORS headers only for the listed origins,
//! preflight before every other check, and a CSRF check that accepts those origins.

mod common;

use ::common::Role;
use common::{Http, ORIGIN, PASSWORD, Resp, db_test, login_app, login_web, seed_user, spawn_with};
use serde_json::json;
use sqlx::PgPool;

const TRUSTED: &str = "https://app.example.edu";
const OTHER: &str = "https://evil.test";

/// The real server, with `TRUSTED` (and a local dev origin) listed, or nothing listed.
async fn server(pool: PgPool, allowed: &[&str]) -> Http {
    let mut settings = common::test_settings();
    settings.server.allowed_origins = allowed.iter().map(|o| (*o).to_owned()).collect();
    Http::new(&spawn_with(pool, settings).await)
}

fn cors_headers(response: &Resp) -> Vec<String> {
    response
        .headers
        .keys()
        .map(|name| name.as_str().to_owned())
        .filter(|name| name.starts_with("access-control-"))
        .collect()
}

async fn preflight(http: &Http, path: &str, origin: &str) -> Resp {
    http.options(path)
        .origin(origin)
        .header("access-control-request-method", "PATCH")
        .header(
            "access-control-request-headers",
            "authorization, content-type",
        )
        .send()
        .await
}

fn list(response: &Resp, name: &str) -> Vec<String> {
    response
        .header(name)
        .split(',')
        .map(|v| v.trim().to_ascii_lowercase())
        .collect()
}

#[tokio::test]
async fn the_default_configuration_sends_no_cors_headers() {
    db_test(async |pool| {
        let http = server(pool.clone(), &[]).await;
        seed_user(&pool, "t@x.edu", Role::Teacher).await;
        let token = login_app(&http, "t@x.edu", PASSWORD).await;

        let requests = [
            preflight(&http, "/courses", TRUSTED).await,
            http.get("/health").origin(TRUSTED).send().await,
            http.get("/me").origin(TRUSTED).bearer(&token).send().await,
            http.post("/courses").origin(TRUSTED).send().await,
        ];
        for response in requests {
            assert_eq!(cors_headers(&response), Vec::<String>::new());
        }
    })
    .await;
}

#[tokio::test]
async fn a_listed_origin_gets_a_preflight_answer_without_a_session() {
    db_test(async |pool| {
        let http = server(pool, &[TRUSTED, "http://localhost:5173"]).await;

        for origin in [TRUSTED, "http://localhost:5173"] {
            let response = preflight(&http, "/courses", origin).await;
            assert!(response.status / 100 == 2, "{}", response.status);
            assert_eq!(response.header("access-control-allow-origin"), origin);
            assert_eq!(response.header("access-control-allow-credentials"), "true");
            assert_eq!(response.header("access-control-max-age"), "600");
            let methods = list(&response, "access-control-allow-methods");
            for method in ["get", "post", "put", "patch", "delete"] {
                assert!(methods.contains(&method.to_owned()), "{methods:?}");
            }
            let headers = list(&response, "access-control-allow-headers");
            assert!(headers.contains(&"authorization".to_owned()), "{headers:?}");
            assert!(headers.contains(&"content-type".to_owned()), "{headers:?}");
            // ADR-0058 and ADR-0059: a cross-origin client may send a key and a tag, and read the tag back.
            for name in ["idempotency-key", "if-none-match"] {
                assert!(headers.contains(&name.to_owned()), "{headers:?}");
            }
        }
    })
    .await;
}

#[tokio::test]
async fn a_preflight_is_never_rate_limited_or_authenticated() {
    db_test(async |pool| {
        let mut settings = common::test_settings();
        settings.server.allowed_origins = vec![TRUSTED.to_owned()];
        settings.auth.login_ip_burst = 1;
        let http = Http::new(&spawn_with(pool, settings).await);

        for _ in 0..5 {
            let response = http
                .options("/auth/login")
                .ip("203.0.113.9")
                .origin(TRUSTED)
                .header("access-control-request-method", "POST")
                .header("access-control-request-headers", "content-type")
                .send()
                .await;
            assert!(response.status / 100 == 2, "{}", response.status);
            assert_eq!(response.header("access-control-allow-origin"), TRUSTED);
        }
    })
    .await;
}

#[tokio::test]
async fn a_listed_origin_sees_the_cors_headers_on_the_real_response_and_on_errors() {
    db_test(async |pool| {
        let http = server(pool.clone(), &[TRUSTED]).await;
        seed_user(&pool, "t@x.edu", Role::Teacher).await;
        let token = login_app(&http, "t@x.edu", PASSWORD).await;

        let ok = http.get("/me").origin(TRUSTED).bearer(&token).send().await;
        assert_eq!(ok.status, 200);
        assert_eq!(ok.header("access-control-allow-origin"), TRUSTED);
        assert_eq!(ok.header("access-control-allow-credentials"), "true");
        assert!(list(&ok, "access-control-expose-headers").contains(&"retry-after".to_owned()));
        assert!(ok.header("vary").to_ascii_lowercase().contains("origin"));

        // The Problem Details rewrite rebuilds some errors; the headers must survive it.
        let unauthenticated = http.get("/me").origin(TRUSTED).send().await;
        unauthenticated.problem(401);
        assert_eq!(
            unauthenticated.header("access-control-allow-origin"),
            TRUSTED
        );
        let missing = http
            .get("/no-such-route")
            .origin(TRUSTED)
            .bearer(&token)
            .send()
            .await;
        missing.problem(404);
        assert_eq!(missing.header("access-control-allow-origin"), TRUSTED);
    })
    .await;
}

#[tokio::test]
async fn an_unlisted_origin_gets_no_allow_origin() {
    db_test(async |pool| {
        let http = server(pool.clone(), &[TRUSTED]).await;
        seed_user(&pool, "t@x.edu", Role::Teacher).await;
        let token = login_app(&http, "t@x.edu", PASSWORD).await;

        for origin in [
            OTHER,
            "https://app.example.edu.evil.test",
            "http://app.example.edu",
        ] {
            let response = preflight(&http, "/courses", origin).await;
            assert!(
                response
                    .headers
                    .get("access-control-allow-origin")
                    .is_none()
            );
            let response = http.get("/me").origin(origin).bearer(&token).send().await;
            assert_eq!(response.status, 200);
            assert!(
                response
                    .headers
                    .get("access-control-allow-origin")
                    .is_none()
            );
        }
        // No Origin at all (a non-browser client) is not a CORS request.
        let response = http.get("/me").bearer(&token).send().await;
        assert!(
            response
                .headers
                .get("access-control-allow-origin")
                .is_none()
        );
    })
    .await;
}

#[tokio::test]
async fn a_cookie_write_from_a_listed_origin_passes_and_from_an_unlisted_one_is_refused() {
    db_test(async |pool| {
        let http = server(pool.clone(), &[TRUSTED]).await;
        seed_user(&pool, "t@x.edu", Role::Teacher).await;

        // A web login is a state change too: it follows the same rule.
        let refused = http
            .post("/auth/login")
            .origin(OTHER)
            .json(&json!({"email": "t@x.edu", "password": PASSWORD, "client": "web"}))
            .send()
            .await;
        refused.problem(403);
        let login = http
            .post("/auth/login")
            .origin(TRUSTED)
            .json(&json!({"email": "t@x.edu", "password": PASSWORD, "client": "web"}))
            .send()
            .await;
        assert_eq!(login.status, 200);
        assert_eq!(login.header("access-control-allow-origin"), TRUSTED);
        let cookie = common::cookie_token(&login);

        let create = |origin: Option<&'static str>| {
            let request = http
                .post("/courses")
                .cookie(&cookie)
                .json(&json!({"title": "From a trusted frontend"}));
            match origin {
                Some(origin) => request.origin(origin),
                None => request,
            }
            .send()
        };
        assert_eq!(create(Some(TRUSTED)).await.status, 201);
        assert_eq!(create(Some(ORIGIN)).await.status, 201);
        for bad in [Some(OTHER), Some("null"), None] {
            let response = create(bad).await;
            let problem = response.problem(403);
            assert_eq!(problem.detail.as_deref(), Some("origin not allowed"));
        }
    })
    .await;
}

#[tokio::test]
async fn a_cookie_write_from_the_public_origin_still_passes_when_nothing_is_listed() {
    db_test(async |pool| {
        let http = server(pool.clone(), &[]).await;
        let password = seed_user(&pool, "t@x.edu", Role::Teacher).await;
        let cookie = login_web(&http, "t@x.edu", &password).await;

        let allowed = http
            .post("/courses")
            .cookie(&cookie)
            .origin(ORIGIN)
            .json(&json!({"title": "Same origin"}))
            .send()
            .await;
        assert_eq!(allowed.status, 201);
        let refused = http
            .post("/courses")
            .cookie(&cookie)
            .origin(TRUSTED)
            .json(&json!({"title": "Cross origin"}))
            .send()
            .await;
        refused.problem(403);
    })
    .await;
}

#[tokio::test]
async fn a_bearer_write_is_not_blocked_by_the_csrf_check() {
    db_test(async |pool| {
        for allowed in [&[][..], &[TRUSTED][..]] {
            let http = server(pool.clone(), allowed).await;
            let email = format!("t{}@x.edu", allowed.len());
            let password = seed_user(&pool, &email, Role::Teacher).await;
            let token = login_app(&http, &email, &password).await;
            for origin in [Some(OTHER), Some(TRUSTED), None] {
                let request = http
                    .post("/courses")
                    .bearer(&token)
                    .json(&json!({"title": "Bearer"}));
                let request = match origin {
                    Some(origin) => request.origin(origin),
                    None => request,
                };
                assert_eq!(request.send().await.status, 201, "{allowed:?} {origin:?}");
            }
        }
    })
    .await;
}
