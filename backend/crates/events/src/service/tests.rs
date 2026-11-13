use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use uuid::Uuid;

use super::*;
use crate::{HandlerError, HandlerFuture, OutboxEvent};
use test_support::db_test;

fn grade_published() -> Event {
    Event::GradePublished {
        assignment_id: Uuid::now_v7(),
        student_id: Uuid::now_v7(),
        course_id: Uuid::now_v7(),
    }
}

/// This test binary never installs a registry, as a process that forgot to would not.
#[tokio::test]
async fn emit_without_an_installed_registry_is_an_error() {
    db_test(async |pool| {
        let mut conn = pool.acquire().await.unwrap();

        let outcome = emit(&mut conn, grade_published()).await;

        assert!(matches!(outcome, Err(EventsError::NoSubscribers)));
    })
    .await;
}

async fn emit_grade(pool: &PgPool) -> Uuid {
    let mut conn = pool.acquire().await.unwrap();
    crate::emit_with(&mut conn, grade_published(), &crate::Subscribers::new())
        .await
        .unwrap();
    sqlx::query_scalar!("SELECT id FROM events.outbox ORDER BY id DESC LIMIT 1")
        .fetch_one(&mut *conn)
        .await
        .unwrap()
}

fn counting(counter: &Arc<AtomicUsize>, fail: bool) -> Handlers {
    let counter = counter.clone();
    Handlers::default().on(
        "grade.published",
        move |_event: OutboxEvent| -> HandlerFuture {
            let counter = counter.clone();
            Box::pin(async move {
                counter.fetch_add(1, Ordering::SeqCst);
                if fail {
                    Err(HandlerError("the mail server said no".to_owned()))
                } else {
                    Ok(())
                }
            })
        },
    )
}

async fn state(pool: &PgPool, id: Uuid) -> (i32, bool, bool, Option<String>) {
    let row = sqlx::query!(
        r#"SELECT attempts, processed_at IS NOT NULL AS "processed!",
                      failed_at IS NOT NULL AS "failed!", last_error
               FROM events.outbox WHERE id = $1"#,
        id
    )
    .fetch_one(pool)
    .await
    .unwrap();
    (row.attempts, row.processed, row.failed, row.last_error)
}

#[tokio::test]
async fn a_handled_event_is_processed_once_and_an_unhandled_one_is_processed_too() {
    db_test(async |pool| {
        let handled = emit_grade(&pool).await;
        let mut conn = pool.acquire().await.unwrap();
        repo::insert_outbox(
            &mut conn,
            &Event::SubmissionSubmitted {
                assignment_id: Uuid::now_v7(),
                student_id: Uuid::now_v7(),
                course_id: Uuid::now_v7(),
                submission_id: Uuid::now_v7(),
            },
        )
        .await
        .unwrap();
        drop(conn);
        let calls = Arc::new(AtomicUsize::new(0));
        let handlers = counting(&calls, false);

        assert_eq!(process_once(&pool, &handlers).await.unwrap(), 2);
        assert_eq!(process_once(&pool, &handlers).await.unwrap(), 0);

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "only grade.published has a handler"
        );
        assert_eq!(state(&pool, handled).await, (1, true, false, None));
        let unprocessed = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM events.outbox WHERE processed_at IS NULL"#
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(unprocessed, 0);
    })
    .await;
}

#[tokio::test]
async fn a_failure_is_retried_later_and_the_fifth_one_makes_a_dead_letter() {
    db_test(async |pool| {
        let id = emit_grade(&pool).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let handlers = counting(&calls, true);

        assert_eq!(process_once(&pool, &handlers).await.unwrap(), 1);
        // Not due yet: the retry waits.
        assert_eq!(process_once(&pool, &handlers).await.unwrap(), 0);
        let (attempts, processed, failed, error) = state(&pool, id).await;
        assert_eq!((attempts, processed, failed), (1, false, false));
        assert_eq!(error.as_deref(), Some("the mail server said no"));

        for expected in 2..=5 {
            sqlx::query!("UPDATE events.outbox SET run_after = now()")
                .execute(&pool)
                .await
                .unwrap();
            assert_eq!(process_once(&pool, &handlers).await.unwrap(), 1);
            assert_eq!(state(&pool, id).await.0, expected);
        }
        let (attempts, processed, failed, _) = state(&pool, id).await;
        assert_eq!((attempts, processed, failed), (5, false, true));
        sqlx::query!("UPDATE events.outbox SET run_after = now()")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            process_once(&pool, &handlers).await.unwrap(),
            0,
            "a dead letter stays"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 5);
    })
    .await;
}

#[tokio::test]
async fn a_claimed_row_is_leased_and_comes_back_when_the_lease_runs_out() {
    db_test(async |pool| {
        let id = emit_grade(&pool).await;
        let mut conn = pool.acquire().await.unwrap();
        let first = repo::claim(&mut conn, 10, 300.0).await.unwrap();
        assert_eq!(first.len(), 1);
        assert!(
            repo::claim(&mut conn, 10, 300.0).await.unwrap().is_empty(),
            "leased"
        );

        // A worker that died: the lease ends, the row is claimed again with one more attempt.
        sqlx::query!("UPDATE events.outbox SET run_after = now() - interval '1 second'")
            .execute(&mut *conn)
            .await
            .unwrap();
        let again = repo::claim(&mut conn, 10, 300.0).await.unwrap();
        assert_eq!((again[0].0.id, again[0].1), (id, 2));
    })
    .await;
}

#[tokio::test]
async fn rows_are_handled_oldest_first_and_two_workers_never_take_the_same_row() {
    db_test(async |pool| {
        let mut ids = Vec::new();
        for _ in 0..12 {
            ids.push(emit_grade(&pool).await);
        }
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorder = seen.clone();
        let handlers = Handlers::default().on(
            "grade.published",
            move |event: OutboxEvent| -> HandlerFuture {
                let recorder = recorder.clone();
                Box::pin(async move {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    recorder.lock().unwrap().push(event.id);
                    Ok(())
                })
            },
        );

        // One worker handles a batch in the order the rows were written.
        assert_eq!(process_once(&pool, &handlers).await.unwrap(), 12);
        assert_eq!(*seen.lock().unwrap(), ids);

        // Two workers at once share the next rows without any row going to both.
        seen.lock().unwrap().clear();
        let mut more = Vec::new();
        for _ in 0..12 {
            more.push(emit_grade(&pool).await);
        }
        let (a, b) = tokio::join!(
            process_once(&pool, &handlers),
            process_once(&pool, &handlers)
        );

        assert_eq!(a.unwrap() + b.unwrap(), 12);
        let mut handled = seen.lock().unwrap().clone();
        handled.sort();
        assert_eq!(handled, more, "each row exactly once");
    })
    .await;
}

#[test]
fn the_wait_doubles_from_ten_seconds_up_to_an_hour() {
    let secs: Vec<u64> = (1..=5).map(|n| backoff(n).as_secs()).collect();
    assert_eq!(secs, [10, 20, 40, 80, 160]);
    assert_eq!(backoff(40), MAX_BACKOFF);
}

#[tokio::test]
async fn several_handlers_of_one_type_all_run_and_one_failure_retries_the_event() {
    db_test(async |pool| {
        let first = Arc::new(AtomicUsize::new(0));
        let second = Arc::new(AtomicUsize::new(0));
        let both = counting(&first, false).merge(counting(&second, true));
        let id = emit_grade(&pool).await;
        process_once(&pool, &both).await.unwrap();
        assert_eq!(first.load(Ordering::SeqCst), 1);
        assert_eq!(second.load(Ordering::SeqCst), 1);
        let (attempts, processed, failed, error) = state(&pool, id).await;
        assert_eq!((attempts, processed, failed), (1, false, false));
        assert!(error.is_some());
    })
    .await;
}

#[test]
fn every_event_has_a_dotted_kind_and_a_payload_of_ids_only() {
    let id = Uuid::now_v7;
    let events = [
        Event::AssignmentCreated {
            course_id: id(),
            assignment_id: id(),
        },
        Event::AssignmentUpdated {
            course_id: id(),
            assignment_id: id(),
        },
        Event::AssignmentDeadlineChanged {
            course_id: id(),
            assignment_id: id(),
        },
        Event::EnrollmentChanged {
            course_id: id(),
            user_id: id(),
        },
        Event::QuizAttemptSubmitted {
            course_id: id(),
            assignment_id: id(),
            student_id: id(),
            submission_id: id(),
        },
        Event::AnnouncementPublished {
            course_id: id(),
            announcement_id: id(),
        },
        Event::GroupMembersChanged {
            group_id: id(),
            user_ids: vec![id(), id()],
        },
        Event::GradePublicationChanged {
            assignment_id: id(),
            student_id: id(),
            course_id: id(),
        },
    ];
    let mut kinds = Vec::new();
    for event in &events {
        kinds.push(event.kind());
        assert_eq!(event.version(), 1);
        let payload = event.payload();
        let fields = payload.as_object().expect("an object");
        assert!(!fields.is_empty());
        // Nothing but ids: a uuid, or a list of them.
        for (name, value) in fields {
            let is_id =
                |v: &serde_json::Value| v.as_str().is_some_and(|t| t.parse::<Uuid>().is_ok());
            let ok = is_id(value) || value.as_array().is_some_and(|list| list.iter().all(is_id));
            assert!(ok, "{} {name}: {value}", event.kind());
        }
    }
    kinds.sort();
    kinds.dedup();
    assert_eq!(kinds.len(), events.len(), "each kind once");
}

#[tokio::test]
async fn a_claimed_event_says_when_it_happened() {
    db_test(async |pool| {
        let before = chrono::Utc::now() - chrono::TimeDelta::seconds(5);
        emit_grade(&pool).await;
        let mut conn = pool.acquire().await.unwrap();
        let claimed = repo::claim(&mut conn, 10, 60.0).await.unwrap();
        assert_eq!(claimed.len(), 1);
        let at = claimed[0].0.occurred_at;
        assert!(at > before && at <= chrono::Utc::now(), "{at}");
    })
    .await;
}

fn audit_entry(resource_id: Uuid) -> AuditEntry {
    AuditEntry {
        action: "course.update",
        resource_type: "course",
        resource_id,
        meta: serde_json::json!({"field": "title"}),
    }
}

fn course_access(
    role: Option<access::CourseRole>,
    unit_grant: Option<access::UnitGrant>,
) -> access::CourseAccess {
    access::CourseAccess {
        role,
        is_owner: false,
        status: access::CourseStatus::Published,
        unit_grant,
    }
}

async fn audit_rows(pool: &PgPool) -> Vec<serde_json::Value> {
    sqlx::query_scalar!("SELECT meta FROM events.audit_log ORDER BY id")
        .fetch_all(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn audit_implicit_writes_for_an_admin_with_via_admin() {
    db_test(async |pool| {
        let admin = test_support::insert_user(&pool, "a@x.edu", "admin").await;
        let actor = Actor::for_tests(admin, common::Role::Admin, Uuid::now_v7());
        let mut conn = pool.acquire().await.unwrap();

        audit_implicit(
            &mut conn,
            &actor,
            course_access(None, None),
            audit_entry(Uuid::now_v7()),
        )
        .await
        .unwrap();

        assert_eq!(
            audit_rows(&pool).await,
            [serde_json::json!({"field": "title", "via": "admin"})]
        );
    })
    .await;
}

#[tokio::test]
async fn audit_implicit_writes_for_a_unit_grant_with_via_and_unit_id() {
    db_test(async |pool| {
        let teacher = test_support::insert_user(&pool, "t@x.edu", "teacher").await;
        let actor = Actor::for_tests(teacher, common::Role::Teacher, Uuid::now_v7());
        let unit_id = Uuid::now_v7();
        let access = course_access(None, Some(access::UnitGrant::full(unit_id)));
        let mut conn = pool.acquire().await.unwrap();

        audit_implicit(&mut conn, &actor, access, audit_entry(Uuid::now_v7()))
            .await
            .unwrap();

        assert_eq!(
            audit_rows(&pool).await,
            [serde_json::json!({"field": "title", "via": "unit", "unit_id": unit_id})]
        );
    })
    .await;
}

#[tokio::test]
async fn audit_implicit_writes_nothing_for_a_course_teacher() {
    db_test(async |pool| {
        let teacher = test_support::insert_user(&pool, "t@x.edu", "teacher").await;
        let actor = Actor::for_tests(teacher, common::Role::Teacher, Uuid::now_v7());
        let unit_id = Uuid::now_v7();
        let mut conn = pool.acquire().await.unwrap();

        // On the course, with or without a unit grant on top: not an implicit way in.
        for access in [
            course_access(Some(access::CourseRole::Teacher), None),
            course_access(
                Some(access::CourseRole::Teacher),
                Some(access::UnitGrant::full(unit_id)),
            ),
        ] {
            audit_implicit(&mut conn, &actor, access, audit_entry(Uuid::now_v7()))
                .await
                .unwrap();
        }

        assert!(audit_rows(&pool).await.is_empty());
    })
    .await;
}

/// A teacher enrolled as a student of a course in their unit writes through the grant, not as
/// staff: the write is audited.
#[tokio::test]
async fn audit_implicit_writes_for_a_grantee_who_is_a_student_of_the_course() {
    db_test(async |pool| {
        let teacher = test_support::insert_user(&pool, "t@x.edu", "teacher").await;
        let actor = Actor::for_tests(teacher, common::Role::Teacher, Uuid::now_v7());
        let unit_id = Uuid::now_v7();
        let access = course_access(
            Some(access::CourseRole::Student),
            Some(access::UnitGrant::full(unit_id)),
        );
        let mut conn = pool.acquire().await.unwrap();

        audit_implicit(&mut conn, &actor, access, audit_entry(Uuid::now_v7()))
            .await
            .unwrap();

        assert_eq!(
            audit_rows(&pool).await,
            [serde_json::json!({"field": "title", "via": "unit", "unit_id": unit_id})]
        );
    })
    .await;
}
