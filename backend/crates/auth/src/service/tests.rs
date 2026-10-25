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
