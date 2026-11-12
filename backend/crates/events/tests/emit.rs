//! `emit` reads the registry the process installed. The registry is global to the process, so this
//! test has a binary of its own.

use std::sync::atomic::{AtomicUsize, Ordering};

use events::{Event, Subscribers, emit, install};
use test_support::db_test;
use uuid::Uuid;

static CALLS: AtomicUsize = AtomicUsize::new(0);

#[tokio::test]
async fn emit_runs_the_installed_subscribers() {
    db_test(async |pool| {
        install(
            Subscribers::new().on("enrollment.changed", |_conn, _event| {
                Box::pin(async {
                    CALLS.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            }),
        )
        .expect("the first install works");
        let mut conn = pool.acquire().await.unwrap();

        emit(
            &mut conn,
            Event::EnrollmentChanged {
                course_id: Uuid::now_v7(),
                user_id: Uuid::now_v7(),
            },
        )
        .await
        .unwrap();

        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    })
    .await;
}
