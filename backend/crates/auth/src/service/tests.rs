use super::*;

fn key(email: &str) -> ThrottleKey {
    ThrottleKey::new(email)
}

/// Counts an attempt and says whether it was refused.
fn refused(throttle: &LoginThrottle, email: &str) -> bool {
    matches!(
        throttle.reserve(&key(email)),
        Err(AuthError::TooManyAttempts { .. })
    )
}

#[test]
fn throttle_allows_ten_attempts_and_refuses_the_eleventh() {
    let throttle = LoginThrottle::default();
    for attempt in 1..=10 {
        assert!(!refused(&throttle, "a@x.edu"), "attempt {attempt}");
    }
    assert!(refused(&throttle, "a@x.edu"));
    assert!(
        !refused(&throttle, "b@x.edu"),
        "other accounts are unaffected"
    );
}

#[test]
fn throttle_counts_attempts_that_are_still_running() {
    // Nothing here ever reports a result: the attempts count when they start, so a burst of
    // parallel guesses is cut off after the limit, not after the hash of each (ADR-0038).
    let throttle = LoginThrottle::with_limits(3, Duration::from_secs(60), 10);
    let allowed = (0..40).filter(|_| !refused(&throttle, "a@x.edu")).count();
    assert_eq!(allowed, 3);
}

#[test]
fn throttle_key_ignores_case_and_surrounding_spaces() {
    let throttle = LoginThrottle::with_limits(1, Duration::from_secs(60), 10);
    assert!(!refused(&throttle, " A@X.edu "));
    assert!(refused(&throttle, "a@x.edu"));
}

#[test]
fn success_resets_the_counter() {
    let throttle = LoginThrottle::with_limits(2, Duration::from_secs(60), 10);
    assert!(!refused(&throttle, "a@x.edu"));
    assert!(!refused(&throttle, "a@x.edu"));
    throttle.reset(&key("a@x.edu"));
    assert!(!refused(&throttle, "a@x.edu"));
    assert!(!refused(&throttle, "a@x.edu"));
    assert!(refused(&throttle, "a@x.edu"));
}

#[test]
fn throttle_window_expires() {
    let throttle = LoginThrottle::with_limits(1, Duration::from_millis(40), 10);
    assert!(!refused(&throttle, "a@x.edu"));
    assert!(refused(&throttle, "a@x.edu"));
    std::thread::sleep(Duration::from_millis(60));
    assert!(!refused(&throttle, "a@x.edu"), "a new window starts");
    assert!(refused(&throttle, "a@x.edu"), "and counts again");
}

#[test]
fn retry_after_is_reported_and_never_underflows() {
    let throttle = LoginThrottle::with_limits(1, Duration::from_secs(60), 10);
    assert!(!refused(&throttle, "a@x.edu"));
    match throttle.reserve(&key("a@x.edu")) {
        Err(AuthError::TooManyAttempts { retry_after_secs }) => {
            assert!((1..=60).contains(&retry_after_secs));
        }
        other => panic!("expected a throttle error, got {other:?}"),
    }
    // A window that ends between the two clock reads of an old version panicked; a window that
    // is already over now is simply a new one.
    let tiny = LoginThrottle::with_limits(1, Duration::from_nanos(1), 10);
    for _ in 0..1_000 {
        assert!(tiny.reserve(&key("a@x.edu")).is_ok());
    }
}

#[test]
fn throttle_memory_is_bounded() {
    let throttle = LoginThrottle::with_limits(10, Duration::from_secs(60), 100);
    for i in 0..1_000 {
        let _ = throttle.reserve(&key(&format!("user{i}@x.edu")));
        assert!(throttle.len() <= 100);
    }
}

#[test]
fn full_throttle_drops_expired_entries_before_live_ones() {
    let throttle = LoginThrottle::with_limits(1, Duration::from_millis(40), 3);
    let _ = throttle.reserve(&key("old1@x.edu"));
    let _ = throttle.reserve(&key("old2@x.edu"));
    std::thread::sleep(Duration::from_millis(60));
    let _ = throttle.reserve(&key("live@x.edu"));
    let _ = throttle.reserve(&key("new@x.edu"));
    assert!(refused(&throttle, "live@x.edu"));
    assert!(refused(&throttle, "new@x.edu"));
    assert_eq!(throttle.len(), 2);
}

#[test]
fn spraying_accounts_cannot_free_a_locked_one() {
    let throttle = LoginThrottle::with_limits(2, Duration::from_secs(60), 10);
    let _ = throttle.reserve(&key("victim@x.edu"));
    let _ = throttle.reserve(&key("victim@x.edu"));
    // Everyone else has used one of two attempts, and the map fills up. The oldest entry is
    // the victim's; the accounts that are not locked out must go first.
    assert!(refused(&throttle, "victim@x.edu"));
    for i in 0..200 {
        let _ = throttle.reserve(&key(&format!("spray{i}@x.edu")));
    }
    assert!(refused(&throttle, "victim@x.edu"), "still locked out");
}

#[tokio::test]
async fn hashes_verify_and_use_owasp_parameters() {
    let gate = HashGate::new(2);
    let hash = hash_password_gated(&gate, "correct horse battery staple")
        .await
        .unwrap();
    assert!(hash.as_str().starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
    assert!(
        verify_password(
            &gate,
            "correct horse battery staple".into(),
            Some(hash.as_str().into())
        )
        .await
        .unwrap()
    );
    assert!(
        !verify_password(&gate, "wrong".into(), Some(hash.as_str().into()))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn the_hash_gate_bounds_how_many_run_at_once() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let gate = std::sync::Arc::new(HashGate::new(2));
    let running = std::sync::Arc::new(AtomicUsize::new(0));
    let highest = std::sync::Arc::new(AtomicUsize::new(0));
    let mut jobs = Vec::new();
    for _ in 0..8 {
        let (gate, running, highest) = (gate.clone(), running.clone(), highest.clone());
        jobs.push(tokio::spawn(async move {
            gate.run(move || {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                highest.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(30));
                running.fetch_sub(1, Ordering::SeqCst);
            })
            .await
        }));
    }
    for job in jobs {
        job.await.unwrap().unwrap();
    }
    assert_eq!(highest.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn verifying_without_a_hash_is_false_but_still_hashes() {
    let started = Instant::now();
    assert!(
        !verify_password(&HashGate::new(1), "anything".into(), None)
            .await
            .unwrap()
    );
    assert!(
        started.elapsed() > Duration::from_millis(1),
        "a dummy hash must have been computed"
    );
}

#[tokio::test]
async fn malformed_stored_hash_is_a_mismatch() {
    assert!(
        !verify_password(
            &HashGate::new(1),
            "pw".into(),
            Some("not a phc string".into())
        )
        .await
        .unwrap()
    );
}

// --- console sessions (ADR-0095) ------------------------------------------------------------

use access::Audience;
use test_support::{db_test, insert_user};

async fn session_of(
    pool: &sqlx::PgPool,
    email: &str,
    role: &str,
    kind: SessionKind,
    audience: Audience,
) -> (Uuid, SessionToken) {
    let id = insert_user(pool, email, role).await;
    let mut conn = pool.acquire().await.unwrap();
    let user = repo::find_user(&mut conn, id).await.unwrap().unwrap();
    let mut tx = pool.begin().await.unwrap();
    let session = create_session_for(&mut tx, &user, kind, audience)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    (id, session.token)
}

async fn session_row(pool: &sqlx::PgPool, token: &SessionToken) -> Uuid {
    sqlx::query_scalar!(
        "SELECT id FROM auth.sessions WHERE token_hash = $1",
        &token.hash()[..]
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_console_session_expires_after_thirty_minutes_idle() {
    db_test(async |pool| {
        let (_, token) = session_of(
            &pool,
            "a@x.edu",
            "admin",
            SessionKind::Web,
            Audience::Console,
        )
        .await;
        let secs = sqlx::query_scalar!(
            r#"SELECT extract(epoch FROM expires_at - now())::float8 AS "s!" FROM auth.sessions"#
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!((29.0 * 60.0..=30.0 * 60.0).contains(&secs), "{secs}");
        let mut conn = pool.acquire().await.unwrap();
        let found = authenticate(&mut conn, &token).await.unwrap();
        assert_eq!(found.audience, Audience::Console);
        sqlx::query!("UPDATE auth.sessions SET expires_at = now() - interval '1 second'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            authenticate(&mut conn, &token).await,
            Err(AuthError::Unauthenticated)
        ));
    })
    .await;
}

#[tokio::test]
async fn a_console_session_slides_but_never_past_eight_hours() {
    db_test(async |pool| {
        let (_, token) = session_of(
            &pool,
            "a@x.edu",
            "admin",
            SessionKind::Web,
            Audience::Console,
        )
        .await;
        let id = session_row(&pool, &token).await;
        sqlx::query!(
            "UPDATE auth.sessions SET created_at = now() - interval '7 hours 59 minutes',
                 last_seen_at = now() - interval '2 minutes'"
        )
        .execute(&pool)
        .await
        .unwrap();
        let mut conn = pool.acquire().await.unwrap();
        let found = authenticate(&mut conn, &token).await.unwrap();
        let (kind, audience) = found
            .touch
            .expect("a console session is refreshed after a minute");
        touch_session(&mut conn, id, kind, audience).await.unwrap();
        let capped = sqlx::query_scalar!(
            r#"SELECT expires_at = created_at + interval '8 hours' AS "capped!"
               FROM auth.sessions WHERE id = $1"#,
            id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(capped, "expires_at slid past created_at + 8 h");
    })
    .await;
}

#[tokio::test]
async fn a_console_session_is_refreshed_after_a_minute_not_an_hour() {
    db_test(async |pool| {
        let (_, console) = session_of(
            &pool,
            "a@x.edu",
            "admin",
            SessionKind::Web,
            Audience::Console,
        )
        .await;
        let (_, app) = session_of(&pool, "b@x.edu", "admin", SessionKind::Web, Audience::App).await;
        let mut conn = pool.acquire().await.unwrap();
        assert!(
            authenticate(&mut conn, &console)
                .await
                .unwrap()
                .touch
                .is_none()
        );
        sqlx::query!("UPDATE auth.sessions SET last_seen_at = now() - interval '2 minutes'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            authenticate(&mut conn, &console)
                .await
                .unwrap()
                .touch
                .is_some()
        );
        assert!(authenticate(&mut conn, &app).await.unwrap().touch.is_none());
        sqlx::query!("UPDATE auth.sessions SET last_seen_at = now() - interval '2 hours'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(authenticate(&mut conn, &app).await.unwrap().touch.is_some());
    })
    .await;
}

#[tokio::test]
async fn a_console_session_of_a_demoted_admin_does_not_resolve() {
    db_test(async |pool| {
        let (id, token) = session_of(
            &pool,
            "a@x.edu",
            "admin",
            SessionKind::Web,
            Audience::Console,
        )
        .await;
        let mut conn = pool.acquire().await.unwrap();
        assert!(authenticate(&mut conn, &token).await.is_ok());
        sqlx::query!("UPDATE auth.users SET role = 'teacher' WHERE id = $1", id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            authenticate(&mut conn, &token).await,
            Err(AuthError::Unauthenticated)
        ));
    })
    .await;
}

#[tokio::test]
async fn the_database_refuses_an_app_client_console_session() {
    db_test(async |pool| {
        let id = insert_user(&pool, "a@x.edu", "admin").await;
        let result = sqlx::query!(
            "INSERT INTO auth.sessions (id, user_id, token_hash, kind, audience, expires_at)
             VALUES ($1, $2, $3, 'app', 'console', now() + interval '1 hour')",
            Uuid::now_v7(),
            id,
            &[7u8; 32][..]
        )
        .execute(&pool)
        .await;
        assert!(result.is_err());
    })
    .await;
}
