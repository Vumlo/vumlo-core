//! Logins under pressure (ADR-0038): a session is never minted from a password that was reset
//! while it was being checked, the per-account limit holds against parallel guesses, and the
//! per-client limit counts an IPv6 /64 as one client and can be configured.

mod common;

use std::time::Duration;

use common::{Env, db_test, wait_until_blocked};
use serde_json::json;
use server::identity::Role;

/// `x-real-ip` is the client address the test configuration trusts.
async fn login(env: &Env, email: &str, password: &str, ip: &str) -> common::Resp {
    env.http
        .post("/auth/login")
        .ip(ip)
        .json(&json!({"email": email, "password": password, "client": "app"}))
        .send()
        .await
}

// --- a session is created only against the credential that was verified (A-03) ---------------

/// The login checked the password, then an admin reset it before the session was written. The
/// password hash is locked and read again in the session's transaction, so the reset waits for that
/// commit and then deletes the session: the token is useless, as the reset promised.
#[tokio::test]
async fn a_session_minted_while_the_password_is_reset_does_not_survive_the_reset() {
    db_test(async |pool| {
        let env = Env::new(pool.clone()).await;
        let admin = env.user("admin@x.edu", Role::Admin).await;
        let victim = env.user("victim@x.edu", Role::Teacher).await;
        // Stretches the insert of a session so the interleaving does not depend on timing; the
        // code under test is the unchanged login.
        sqlx::query(
            "CREATE FUNCTION slow_insert() RETURNS trigger LANGUAGE plpgsql AS
             $$ BEGIN PERFORM pg_sleep(1.5); RETURN NEW; END $$",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TRIGGER slow BEFORE INSERT ON identity.sessions
             FOR EACH ROW EXECUTE FUNCTION slow_insert()",
        )
        .execute(&pool)
        .await
        .unwrap();

        let base = env.base.clone();
        let attacker = tokio::spawn(async move {
            common::Http::new(&base)
                .post("/auth/login")
                .json(&json!({"email": "victim@x.edu", "password": common::PASSWORD, "client": "app"}))
                .send()
                .await
        });
        // The password check is done and the session insert is running: the user row is locked.
        let waiting = tokio::spawn({
            let (env_base, token, id) = (env.base.clone(), admin.token.clone(), victim.id);
            async move {
                common::Http::new(&env_base)
                    .post(&format!("/admin/users/{id}/reset-password"))
                    .bearer(&token)
                    .send()
                    .await
            }
        });
        // Give the login the head start it needs to reach the insert before the reset starts.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let reset = waiting.await.unwrap();
        assert_eq!(reset.status, 200);
        let minted = attacker.await.unwrap();
        // Whether the reset ran before or after the login's transaction, no session survives it.
        if minted.status == 200 {
            let token = minted.json()["token"].as_str().unwrap().to_owned();
            let me = env.call("GET", &token, "/me", None).await;
            assert_eq!(me.status, 401, "the reset must have deleted the new session");
        } else {
            assert_eq!(minted.status, 401);
        }
        let live: i64 = sqlx::query_scalar("SELECT count(*) FROM identity.sessions WHERE user_id = $1")
            .bind(victim.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(live, 0, "no session of the reset user is left");
    })
    .await;
}

/// The other order: the reset has changed the password but not committed when the login, which
/// checked the old password, goes to write its session. It waits for the reset and then refuses.
#[tokio::test]
async fn a_login_that_verified_a_password_the_reset_replaced_is_refused() {
    db_test(async |pool| {
        let env = Env::new(pool.clone()).await;
        let victim = env.user("victim@x.edu", Role::Teacher).await;
        let mut reset = pool.begin().await.unwrap();
        sqlx::query("UPDATE identity.identities SET password_hash = password_hash || 'x' WHERE user_id = $1")
            .bind(victim.id)
            .execute(&mut *reset)
            .await
            .unwrap();
        // The login reads the committed hash, so the password still checks out.
        let attempt = tokio::spawn({
            let base = env.base.clone();
            async move {
                common::Http::new(&base)
                    .post("/auth/login")
                    .json(&json!({"email": "victim@x.edu", "password": common::PASSWORD, "client": "app"}))
                    .send()
                    .await
            }
        });
        wait_until_blocked(&pool, &attempt).await;
        sqlx::query("DELETE FROM identity.sessions WHERE user_id = $1")
            .bind(victim.id)
            .execute(&mut *reset)
            .await
            .unwrap();
        reset.commit().await.unwrap();
        let outcome = attempt.await.unwrap();
        assert_eq!(outcome.problem(401).status, 401);
    })
    .await;
}

#[tokio::test]
async fn a_login_that_verified_an_account_that_was_disabled_meanwhile_is_refused() {
    db_test(async |pool| {
        let env = Env::new(pool.clone()).await;
        let victim = env.user("victim@x.edu", Role::Teacher).await;
        // The seeding login left one session; the refused login must not add another.
        let before: i64 =
            sqlx::query_scalar("SELECT count(*) FROM identity.sessions WHERE user_id = $1")
                .bind(victim.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let mut disabling = pool.begin().await.unwrap();
        sqlx::query("UPDATE identity.users SET status = 'disabled' WHERE id = $1")
            .bind(victim.id)
            .execute(&mut *disabling)
            .await
            .unwrap();
        let attempt = tokio::spawn({
            let base = env.base.clone();
            async move {
                common::Http::new(&base)
                    .post("/auth/login")
                    .json(&json!({"email": "victim@x.edu", "password": common::PASSWORD, "client": "app"}))
                    .send()
                    .await
            }
        });
        wait_until_blocked(&pool, &attempt).await;
        disabling.commit().await.unwrap();
        assert_eq!(attempt.await.unwrap().problem(401).status, 401);
        let sessions: i64 =
            sqlx::query_scalar("SELECT count(*) FROM identity.sessions WHERE user_id = $1")
                .bind(victim.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(sessions, before, "not even an inert session for a disabled account");
    })
    .await;
}

// --- the per-account limit counts attempts when they start (A-04) ----------------------------

#[tokio::test]
async fn parallel_guesses_cannot_get_past_the_account_limit() {
    db_test(async |pool| {
        let env = Env::new(pool.clone()).await;
        env.user("victim@x.edu", Role::Teacher).await;
        let mut guesses = Vec::new();
        for i in 0..40 {
            let base = env.base.clone();
            guesses.push(tokio::spawn(async move {
                common::Http::new(&base)
                    .post("/auth/login")
                    .ip(&format!("10.0.0.{i}"))
                    .json(&json!({"email": "victim@x.edu", "password": format!("wrong-{i}"), "client": "app"}))
                    .send()
                    .await
                    .status
            }));
        }
        let mut statuses = Vec::new();
        for guess in guesses {
            statuses.push(guess.await.unwrap());
        }
        let evaluated = statuses.iter().filter(|s| **s == 401).count();
        let refused = statuses.iter().filter(|s| **s == 429).count();
        assert_eq!(evaluated, 10, "{statuses:?}");
        assert_eq!(refused, 30, "{statuses:?}");
    })
    .await;
}

// --- the per-client limit (A-09) --------------------------------------------------------------

fn settings_with(burst: u32) -> vumlo_config::Settings {
    let mut settings = common::test_settings();
    settings.auth.login_ip_burst = burst;
    // Slow enough that nothing is replenished while the test runs.
    settings.auth.login_ip_replenish_seconds = 3600;
    settings
}

#[tokio::test]
async fn the_per_client_limit_is_configurable_and_counts_per_ip() {
    db_test(async |pool| {
        let env = Env::with_settings(pool.clone(), settings_with(3)).await;
        env.user("a@x.edu", Role::Teacher).await;
        // `env.user` logged in once from the default address; use other clients here.
        for attempt in 1..=3 {
            let r = login(&env, "a@x.edu", common::PASSWORD, "192.0.2.1").await;
            assert_eq!(r.status, 200, "attempt {attempt}");
        }
        let fourth = login(&env, "a@x.edu", common::PASSWORD, "192.0.2.1").await;
        assert_eq!(fourth.problem(429).status, 429);
        assert!(fourth.headers.contains_key("retry-after"));
        // Another client is not affected.
        assert_eq!(
            login(&env, "a@x.edu", common::PASSWORD, "192.0.2.2")
                .await
                .status,
            200
        );
    })
    .await;
}

#[tokio::test]
async fn an_ipv6_slash_64_is_one_client() {
    db_test(async |pool| {
        let env = Env::with_settings(pool.clone(), settings_with(2)).await;
        env.user("a@x.edu", Role::Teacher).await;
        // Whoever holds a /64 chooses the rest of the address freely; they still share one allowance.
        assert_eq!(
            login(&env, "a@x.edu", "wrong", "2001:db8:1:2::1")
                .await
                .status,
            401
        );
        assert_eq!(
            login(&env, "a@x.edu", "wrong", "2001:db8:1:2:aaaa::2")
                .await
                .status,
            401
        );
        assert_eq!(
            login(&env, "a@x.edu", "wrong", "2001:db8:1:2:ffff:ffff:ffff:ffff")
                .await
                .status,
            429
        );
        // The next /64 is another client.
        assert_eq!(
            login(&env, "a@x.edu", "wrong", "2001:db8:1:3::1")
                .await
                .status,
            401
        );
    })
    .await;
}

#[tokio::test]
async fn a_hash_gate_of_one_still_serves_every_login() {
    db_test(async |pool| {
        let mut settings = common::test_settings();
        settings.auth.max_concurrent_hashes = 1;
        let env = Env::with_settings(pool.clone(), settings).await;
        env.user("a@x.edu", Role::Teacher).await;
        let mut logins = Vec::new();
        for i in 0..6 {
            let base = env.base.clone();
            logins.push(tokio::spawn(async move {
                common::Http::new(&base)
                    .post("/auth/login")
                    .ip(&format!("10.2.0.{i}"))
                    .json(
                        &json!({"email": "a@x.edu", "password": common::PASSWORD, "client": "app"}),
                    )
                    .send()
                    .await
                    .status
            }));
        }
        for login in logins {
            assert_eq!(login.await.unwrap(), 200);
        }
    })
    .await;
}
