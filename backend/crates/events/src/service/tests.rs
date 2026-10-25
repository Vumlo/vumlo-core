use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use uuid::Uuid;

use super::*;
use crate::{HandlerError, HandlerFuture, OutboxEvent};
use test_support::db_test;

async fn emit_grade(pool: &PgPool) -> Uuid {
    let mut conn = pool.acquire().await.unwrap();
    emit(
        &mut conn,
        Event::GradePublished {
            assignment_id: Uuid::now_v7(),
            student_id: Uuid::now_v7(),
            course_id: Uuid::now_v7(),
        },
    )
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
