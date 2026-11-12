use std::sync::{Arc, Mutex};

use uuid::Uuid;

use super::*;
use test_support::db_test;

fn grade() -> Event {
    Event::GradePublished {
        assignment_id: Uuid::now_v7(),
        student_id: Uuid::now_v7(),
        course_id: Uuid::now_v7(),
    }
}

fn enrollment() -> Event {
    Event::EnrollmentChanged {
        course_id: Uuid::now_v7(),
        user_id: Uuid::now_v7(),
    }
}

/// A subscriber that records `mark`, and fails when `fail` is set.
fn recording(log: &Arc<Mutex<Vec<u8>>>, mark: u8, fail: bool) -> Subscribers {
    let log = log.clone();
    Subscribers::new().on("grade.published", move |_conn, _event| {
        let log = log.clone();
        Box::pin(async move {
            log.lock().expect("the log is not poisoned").push(mark);
            if fail {
                Err(SubscriberError("refused".to_owned()))
            } else {
                Ok(())
            }
        })
    })
}

async fn outbox_rows(conn: &mut PgConnection) -> i64 {
    sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM events.outbox"#)
        .fetch_one(conn)
        .await
        .unwrap()
}

#[tokio::test]
async fn subscribers_run_in_registration_order_after_the_outbox_row_is_written() {
    db_test(async |pool| {
        let log = Arc::new(Mutex::new(Vec::new()));
        let first = {
            let log = log.clone();
            Subscribers::new().on("grade.published", move |conn, _event| {
                let log = log.clone();
                Box::pin(async move {
                    // The row of the event is already in the caller's transaction.
                    let rows = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM events.outbox"#)
                        .fetch_one(conn)
                        .await
                        .map_err(|e| SubscriberError(e.to_string()))?;
                    assert_eq!(rows, 1);
                    log.lock().unwrap().push(1);
                    Ok(())
                })
            })
        };
        let subscribers = first.merge(recording(&log, 2, false));
        let mut conn = pool.acquire().await.unwrap();

        emit_with(&mut conn, grade(), &subscribers).await.unwrap();

        assert_eq!(*log.lock().unwrap(), vec![1, 2]);
    })
    .await;
}

#[tokio::test]
async fn a_subscriber_of_another_kind_is_not_run() {
    db_test(async |pool| {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut conn = pool.acquire().await.unwrap();

        emit_with(&mut conn, enrollment(), &recording(&log, 1, false))
            .await
            .unwrap();

        assert!(log.lock().unwrap().is_empty());
        assert_eq!(outbox_rows(&mut conn).await, 1);
    })
    .await;
}

#[tokio::test]
async fn a_subscriber_error_is_returned_and_the_caller_can_roll_back() {
    db_test(async |pool| {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut tx = pool.begin().await.unwrap();

        let outcome = emit_with(&mut tx, grade(), &recording(&log, 1, true)).await;

        assert!(matches!(outcome, Err(EventsError::Subscriber(_))));
        tx.rollback().await.unwrap();
        let mut conn = pool.acquire().await.unwrap();
        assert_eq!(outbox_rows(&mut conn).await, 0);
    })
    .await;
}

#[tokio::test]
async fn the_second_subscriber_does_not_run_after_the_first_failed() {
    db_test(async |pool| {
        let log = Arc::new(Mutex::new(Vec::new()));
        let subscribers = recording(&log, 1, true).merge(recording(&log, 2, false));
        let mut conn = pool.acquire().await.unwrap();

        assert!(emit_with(&mut conn, grade(), &subscribers).await.is_err());

        assert_eq!(*log.lock().unwrap(), vec![1]);
    })
    .await;
}
