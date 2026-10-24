use super::*;
use crate::test_support::{db_test, wait_until_blocked};

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

async fn admin_actor(pool: &sqlx::PgPool, email: &str) -> Actor {
    use super::super::model::{Email, PersonName};
    let user = repo::insert_user(
        &mut pool.acquire().await.unwrap(),
        Uuid::now_v7(),
        &NewUser {
            email: Email::parse(email).unwrap(),
            name: PersonName::name("Admin").unwrap(),
            surname: PersonName::surname("Admin").unwrap(),
            role: Role::Admin,
        },
        false,
    )
    .await
    .unwrap();
    Actor::new(user.id, Role::Admin, Uuid::now_v7())
}

fn demote() -> UserUpdate {
    UserUpdate {
        role: Some(Role::Teacher),
        ..UserUpdate::default()
    }
}

#[tokio::test]
async fn concurrent_demotions_cannot_remove_every_admin() {
    db_test(async |pool| {
        let a = admin_actor(&pool, "a@x.edu").await;
        let b = admin_actor(&pool, "b@x.edu").await;

        // `a` demotes `b` and holds its transaction open; `b` tries to demote `a` meanwhile.
        let mut first = pool.begin().await.unwrap();
        update_user(&mut first, &a, b.user_id(), demote())
            .await
            .unwrap();
        let second = tokio::spawn({
            let pool = pool.clone();
            let (b, a_id) = (b.clone(), a.user_id());
            async move {
                let mut tx = pool.begin().await.unwrap();
                update_user(&mut tx, &b, a_id, demote()).await
            }
        });
        wait_until_blocked(&pool, &second).await;
        first.commit().await.unwrap();

        let outcome = second.await.unwrap();
        assert!(matches!(outcome, Err(AuthError::LastAdmin)), "{outcome:?}");
        let admins = repo::lock_active_admins(&mut pool.acquire().await.unwrap())
            .await
            .unwrap();
        assert_eq!(admins, [a.user_id()]);
    })
    .await;
}

fn to_student() -> UserUpdate {
    UserUpdate {
        role: Some(Role::Student),
        ..UserUpdate::default()
    }
}

async fn teacher_actor(pool: &sqlx::PgPool, email: &str) -> Actor {
    use super::super::model::{Email, PersonName};
    let user = repo::insert_user(
        &mut pool.acquire().await.unwrap(),
        Uuid::now_v7(),
        &NewUser {
            email: Email::parse(email).unwrap(),
            name: PersonName::name("Teacher").unwrap(),
            surname: PersonName::surname("Teacher").unwrap(),
            role: Role::Teacher,
        },
        false,
    )
    .await
    .unwrap();
    Actor::new(user.id, Role::Teacher, Uuid::now_v7())
}

async fn course_of(pool: &sqlx::PgPool, owner: &Actor) -> Uuid {
    let mut tx = pool.begin().await.unwrap();
    let course = teaching::create_course(
        &mut tx,
        owner,
        teaching::CourseTitle::parse("Algebra").unwrap(),
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    course.id
}

/// A member write holds the user's row `FOR SHARE` from the moment it reads the role; the
/// demotion that comes meanwhile waits, and then removes the membership the write added.
#[tokio::test]
async fn a_demotion_waits_for_a_member_write_and_then_removes_the_membership() {
    db_test(async |pool| {
        let admin = admin_actor(&pool, "a@x.edu").await;
        let owner = teacher_actor(&pool, "owner@x.edu").await;
        let candidate = teacher_actor(&pool, "candidate@x.edu").await;
        let course = course_of(&pool, &owner).await;

        let mut first = pool.begin().await.unwrap();
        teaching::put_member(
            &mut first,
            &owner,
            course,
            candidate.user_id(),
            crate::access::CourseRole::Teacher,
        )
        .await
        .unwrap();
        let second = tokio::spawn({
            let pool = pool.clone();
            let (admin, who) = (admin.clone(), candidate.user_id());
            async move {
                let mut tx = pool.begin().await.unwrap();
                update_user(&mut tx, &admin, who, to_student()).await?;
                tx.commit().await.map_err(AuthError::from)
            }
        });
        wait_until_blocked(&pool, &second).await;
        first.commit().await.unwrap();
        second.await.unwrap().unwrap();

        // The course is a draft: only a member who is staff can still see it.
        let access = teaching::course_access(
            &mut pool.acquire().await.unwrap(),
            &Actor::new(candidate.user_id(), Role::Student, Uuid::now_v7()),
            course,
        )
        .await;
        assert!(
            matches!(access, Err(TeachingError::CourseNotFound)),
            "no teacher membership survives the demotion: {access:?}"
        );
    })
    .await;
}

/// The other order: the demotion is in flight, and a member write that comes meanwhile waits
/// for it and then reads the new role.
#[tokio::test]
async fn a_member_write_waits_for_a_demotion_and_then_refuses_the_teacher_role() {
    db_test(async |pool| {
        let admin = admin_actor(&pool, "a@x.edu").await;
        let owner = teacher_actor(&pool, "owner@x.edu").await;
        let candidate = teacher_actor(&pool, "candidate@x.edu").await;
        let course = course_of(&pool, &owner).await;

        let mut first = pool.begin().await.unwrap();
        update_user(&mut first, &admin, candidate.user_id(), to_student())
            .await
            .unwrap();
        let second = tokio::spawn({
            let pool = pool.clone();
            let (owner, who) = (owner.clone(), candidate.user_id());
            async move {
                let mut tx = pool.begin().await.unwrap();
                teaching::put_member(
                    &mut tx,
                    &owner,
                    course,
                    who,
                    crate::access::CourseRole::Teacher,
                )
                .await
            }
        });
        wait_until_blocked(&pool, &second).await;
        first.commit().await.unwrap();

        let outcome = second.await.unwrap();
        assert!(
            matches!(outcome, Err(TeachingError::RoleNotAllowed)),
            "{outcome:?}"
        );
    })
    .await;
}
