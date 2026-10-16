//! The installation record (ADR-0031): the first start records the mode, later starts must agree.

use std::time::Duration;

use server::org::{self, OrgError};
use server::settings::InstitutionMode::{self, School, University};
use sqlx::PgPool;

async fn start(pool: &PgPool, mode: InstitutionMode) -> Result<(), OrgError> {
    let mut tx = pool.begin().await.unwrap();
    org::record_mode(&mut tx, mode).await?;
    tx.commit().await.unwrap();
    Ok(())
}

async fn recorded(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar!("SELECT mode FROM org.installation")
        .fetch_all(pool)
        .await
        .unwrap()
}

/// Waits until some other session of this database is blocked on a lock. That is the second
/// starter waiting for the first one's uncommitted row, so the race is really exercised and
/// the steps do not depend on timing.
async fn wait_for_a_blocked_session(pool: &PgPool) {
    for _ in 0..200 {
        let blocked = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM pg_stat_activity
               WHERE datname = current_database() AND wait_event_type = 'Lock'"#
        )
        .fetch_one(pool)
        .await
        .unwrap();
        if blocked > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the second starter never blocked on the first");
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn the_first_start_records_the_mode(pool: PgPool) {
    assert!(recorded(&pool).await.is_empty());

    start(&pool, School).await.unwrap();

    assert_eq!(recorded(&pool).await, ["school"]);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_restart_with_the_same_mode_passes_and_changes_nothing(pool: PgPool) {
    start(&pool, University).await.unwrap();
    start(&pool, University).await.unwrap();

    assert_eq!(recorded(&pool).await, ["university"]);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_different_mode_is_refused_and_the_message_names_both_values(pool: PgPool) {
    start(&pool, School).await.unwrap();

    let error = start(&pool, University).await.unwrap_err();

    assert!(
        matches!(
            error,
            OrgError::ModeChanged {
                recorded: School,
                configured: University
            }
        ),
        "{error}"
    );
    let message = error.to_string();
    for needle in ["institution.mode", "school", "university"] {
        assert!(message.contains(needle), "{needle}: {message}");
    }
    assert_eq!(recorded(&pool).await, ["school"]);
}

/// A records and has not committed; B starts meanwhile, has to wait for A's row, and is let go
/// when A commits. Returns B's outcome; A must always pass.
async fn second_starter_passes(
    pool: &PgPool,
    first: InstitutionMode,
    second: InstitutionMode,
) -> bool {
    let mut a = pool.begin().await.unwrap();
    org::record_mode(&mut a, first).await.unwrap();

    let b = tokio::spawn({
        let pool = pool.clone();
        async move { start(&pool, second).await.is_ok() }
    });
    wait_for_a_blocked_session(pool).await;
    a.commit().await.unwrap();

    b.await.unwrap()
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn two_first_starts_with_the_same_mode_both_pass(pool: PgPool) {
    assert!(second_starter_passes(&pool, School, School).await);
    assert_eq!(recorded(&pool).await, ["school"]);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn two_first_starts_with_different_modes_exactly_one_passes(pool: PgPool) {
    assert!(!second_starter_passes(&pool, School, University).await);
    assert_eq!(recorded(&pool).await, ["school"]);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_starter_that_rolls_back_does_not_block_the_next_one(pool: PgPool) {
    let mut a = pool.begin().await.unwrap();
    org::record_mode(&mut a, School).await.unwrap();
    a.rollback().await.unwrap();

    start(&pool, University).await.unwrap();

    assert_eq!(recorded(&pool).await, ["university"]);
}
