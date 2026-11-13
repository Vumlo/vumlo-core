//! The admin console's door (ADR-0095): its login, its cookie, the channels a credential may use,
//! and what a session of one audience cannot do with the other's.

mod common;

use ::common::Role;
use common::{ADMIN_ORIGIN, Env, ORIGIN, PASSWORD, db_test};
use serde_json::json;
use sqlx::PgPool;

async fn console_login(
    env: &Env,
    email: &str,
    password: &str,
    origin: Option<&str>,
) -> common::Resp {
    let mut request = env.http.post("/console/login");
    if let Some(origin) = origin {
        request = request.origin(origin);
    }
    request
        .json(&json!({"email": email, "password": password}))
        .send()
        .await
}

async fn app_login(env: &Env, email: &str, password: &str) -> common::Resp {
    env.http
        .post("/auth/login")
        .json(&json!({"email": email, "password": password, "client": "app"}))
        .send()
        .await
}

async fn sessions_of(pool: &PgPool, email: &str) -> i64 {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM auth.sessions s JOIN auth.users u ON u.id = s.user_id
           WHERE u.email = $1"#,
        email
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn an_admin_signs_in_to_the_console_and_gets_the_console_cookie() {
    db_test(async |pool| {
        let env = Env::new(pool.clone()).await;
        common::seed_user(&pool, "admin@x.edu", Role::Admin).await;
        let response = console_login(&env, "admin@x.edu", PASSWORD, Some(ADMIN_ORIGIN)).await;
        assert_eq!(
            response.status,
            200,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        let cookie = response.header("set-cookie");
        assert!(cookie.starts_with("vumlo_console="), "{cookie}");
        for part in ["HttpOnly", "SameSite=Strict", "Path=/", "Secure"] {
            assert!(cookie.contains(part), "{part} missing in {cookie}");
        }
        assert!(!cookie.to_lowercase().contains("domain"), "{cookie}");
        assert_eq!(response.header("cache-control"), "no-store");
        assert_eq!(response.json()["role"], "admin");
        let row = sqlx::query!("SELECT kind, audience FROM auth.sessions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            (row.kind.as_str(), row.audience.as_str()),
            ("web", "console")
        );
    })
    .await;
}

#[tokio::test]
async fn a_wrong_password_an_unknown_email_and_a_non_admin_look_the_same() {
    db_test(async |pool| {
        let env = Env::new(pool.clone()).await;
        common::seed_user(&pool, "admin@x.edu", Role::Admin).await;
        common::seed_user(&pool, "teacher@x.edu", Role::Teacher).await;
        let wrong =
            console_login(&env, "admin@x.edu", "not the password", Some(ADMIN_ORIGIN)).await;
        let unknown = console_login(&env, "nobody@x.edu", PASSWORD, Some(ADMIN_ORIGIN)).await;
        // The right password of an account that is not an administrator.
        let teacher = console_login(&env, "teacher@x.edu", PASSWORD, Some(ADMIN_ORIGIN)).await;
        for response in [&wrong, &unknown, &teacher] {
            assert_eq!(response.status, 401);
            assert!(response.headers.get("set-cookie").is_none());
        }
        assert_eq!(wrong.body, unknown.body);
        assert_eq!(wrong.body, teacher.body);
        assert_eq!(sessions_of(&pool, "teacher@x.edu").await, 0);
    })
    .await;
}

#[tokio::test]
async fn the_console_login_needs_the_admin_origin() {
    db_test(async |pool| {
        let env = Env::new(pool.clone()).await;
        common::seed_user(&pool, "admin@x.edu", Role::Admin).await;
        for origin in [Some(ORIGIN), Some("https://evil.test"), None] {
            let response = console_login(&env, "admin@x.edu", PASSWORD, origin).await;
            assert_eq!(response.status, 403, "{origin:?}");
        }
        assert_eq!(sessions_of(&pool, "admin@x.edu").await, 0);
    })
    .await;
}

#[tokio::test]
async fn console_failures_do_not_lock_the_app_login_and_the_reverse() {
    db_test(async |pool| {
        let env = Env::new(pool.clone()).await;
        common::seed_user(&pool, "admin@x.edu", Role::Admin).await;
        for _ in 0..10 {
            let response = console_login(&env, "admin@x.edu", "wrong", Some(ADMIN_ORIGIN)).await;
            assert_eq!(response.status, 401);
        }
        let locked = console_login(&env, "admin@x.edu", PASSWORD, Some(ADMIN_ORIGIN)).await;
        assert_eq!(locked.status, 429);
        assert_eq!(app_login(&env, "admin@x.edu", PASSWORD).await.status, 200);

        common::seed_user(&pool, "other@x.edu", Role::Admin).await;
        for _ in 0..10 {
            assert_eq!(app_login(&env, "other@x.edu", "wrong").await.status, 401);
        }
        assert_eq!(app_login(&env, "other@x.edu", PASSWORD).await.status, 429);
        let response = console_login(&env, "other@x.edu", PASSWORD, Some(ADMIN_ORIGIN)).await;
        assert_eq!(response.status, 200);
    })
    .await;
}

#[tokio::test]
async fn a_temporary_password_admin_can_only_change_it() {
    db_test(async |pool| {
        let env = Env::new(pool.clone()).await;
        let temporary = common::seed_pending_user(&pool, "admin@x.edu", Role::Admin).await;
        let token = common::login_console(&env.http, "admin@x.edu", &temporary).await;
        let me = env.http.get("/me").console(&token).send().await;
        assert_eq!(me.status, 200);
        assert_eq!(me.json()["must_change_password"], true);
        let users = env.http.get("/admin/users").console(&token).send().await;
        assert_eq!(users.status, 403);
        let detail = users.problem(403).detail.unwrap_or_default();
        assert!(detail.contains("password must be changed"), "{detail}");
    })
    .await;
}

#[tokio::test]
async fn logout_clears_the_console_cookie_and_deletes_the_session() {
    db_test(async |pool| {
        let env = Env::new(pool.clone()).await;
        common::seed_user(&pool, "admin@x.edu", Role::Admin).await;
        let token = common::login_console(&env.http, "admin@x.edu", PASSWORD).await;
        let out = env.http.post("/auth/logout").console(&token).send().await;
        assert_eq!(out.status, 204);
        let cookie = out.header("set-cookie");
        assert!(cookie.starts_with("vumlo_console=;"), "{cookie}");
        assert!(cookie.contains("Max-Age=0"), "{cookie}");
        assert_eq!(env.http.get("/me").console(&token).send().await.status, 401);
    })
    .await;
}

#[tokio::test]
async fn a_console_token_is_not_a_bearer_token_and_not_an_app_cookie() {
    db_test(async |pool| {
        let env = Env::new(pool.clone()).await;
        common::seed_user(&pool, "admin@x.edu", Role::Admin).await;
        let console = common::login_console(&env.http, "admin@x.edu", PASSWORD).await;
        let app = common::login_app(&env.http, "admin@x.edu", PASSWORD).await;
        assert_eq!(
            env.http.get("/me").console(&console).send().await.status,
            200
        );
        assert_eq!(
            env.http.get("/me").bearer(&console).send().await.status,
            401
        );
        assert_eq!(
            env.http.get("/me").cookie(&console).send().await.status,
            401
        );
        assert_eq!(env.http.get("/me").bearer(&app).send().await.status, 200);
        let app_as_console = env
            .http
            .get("/me")
            .header("cookie", &format!("vumlo_console={app}"))
            .send()
            .await;
        assert_eq!(app_as_console.status, 401);
    })
    .await;
}

#[tokio::test]
async fn a_demoted_admin_loses_the_console_session_at_once() {
    db_test(async |pool| {
        let env = Env::new(pool.clone()).await;
        common::seed_user(&pool, "admin@x.edu", Role::Admin).await;
        common::seed_user(&pool, "second@x.edu", Role::Admin).await;
        let token = common::login_console(&env.http, "admin@x.edu", PASSWORD).await;
        assert_eq!(env.http.get("/me").console(&token).send().await.status, 200);
        sqlx::query!("UPDATE auth.users SET role = 'teacher' WHERE email = 'admin@x.edu'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(env.http.get("/me").console(&token).send().await.status, 401);
    })
    .await;
}

#[tokio::test]
async fn the_console_cookie_does_not_work_for_a_write_from_the_app_origin() {
    db_test(async |pool| {
        let env = Env::new(pool.clone()).await;
        common::seed_user(&pool, "admin@x.edu", Role::Admin).await;
        let teacher = common::seed_and_login(&pool, &env.http, "t@x.edu", Role::Teacher)
            .await
            .0;
        let path = format!("/admin/users/{teacher}/reset-password");
        let console = common::login_console(&env.http, "admin@x.edu", PASSWORD).await;
        let app = common::login_web(&env.http, "admin@x.edu", PASSWORD).await;
        let both = format!("vumlo_console={console}; vumlo_session={app}");

        // The console cookie alone, from the app's origin: not a credential.
        let alone = env
            .http
            .post(&path)
            .header("cookie", &format!("vumlo_console={console}"))
            .origin(ORIGIN)
            .send()
            .await;
        assert_eq!(alone.status, 401);
        // Both cookies from the app's origin: the app session speaks, and it may not administer.
        let from_app = env
            .http
            .post(&path)
            .header("cookie", &both)
            .origin(ORIGIN)
            .send()
            .await;
        assert_eq!(from_app.status, 403);
        // From the console's origin the console session speaks.
        let from_console = env
            .http
            .post(&path)
            .header("cookie", &both)
            .origin(ADMIN_ORIGIN)
            .send()
            .await;
        assert_eq!(
            from_console.status,
            200,
            "{}",
            String::from_utf8_lossy(&from_console.body)
        );
    })
    .await;
}

#[tokio::test]
async fn a_dead_console_cookie_does_not_shadow_the_app_cookie_beside_it() {
    db_test(async |pool| {
        let env = Env::new(pool.clone()).await;
        common::seed_user(&pool, "admin@x.edu", Role::Admin).await;
        let console = common::login_console(&env.http, "admin@x.edu", PASSWORD).await;
        let app = common::login_web(&env.http, "admin@x.edu", PASSWORD).await;
        sqlx::query!("UPDATE auth.sessions SET expires_at = now() - interval '1 second' WHERE audience = 'console'")
            .execute(&pool)
            .await
            .unwrap();
        let me = env
            .http
            .get("/me")
            .header("cookie", &format!("vumlo_console={console}; vumlo_session={app}"))
            .send()
            .await;
        assert_eq!(me.status, 200);
    })
    .await;
}
