use std::time::Duration;

use sqlx::{PgConnection, PgPool};

use super::model::{AuditEntry, AuditFilter, AuditRow, Event, Handlers};
use super::repo;
use crate::actor::Actor;
use crate::policy::{self, Action, PolicyError, implicit_course_owner};

#[derive(Debug, thiserror::Error)]
pub enum EventsError {
    #[error(transparent)]
    Forbidden(#[from] PolicyError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// Records that `actor` did something, on the caller's connection so the entry commits or rolls
/// back with the change it describes. Callers decide what is audited (AGENTS §5); this does not
/// authorize anything.
pub async fn audit(
    conn: &mut PgConnection,
    actor: &Actor,
    entry: AuditEntry,
) -> Result<(), EventsError> {
    repo::insert_audit(conn, Some(actor.user_id()), "api", &entry).await?;
    Ok(())
}

/// The rows of the audit log, newest first, paged by id (ADR-0053). Admins only.
pub async fn list_audit(
    conn: &mut PgConnection,
    actor: &Actor,
    mut filter: AuditFilter,
    before: Option<uuid::Uuid>,
    limit: i64,
) -> Result<Vec<AuditRow>, EventsError> {
    policy::authorize(actor, Action::ViewAdminReports)?;
    let pattern = filter.text.take().map(|text| like_pattern(text.trim()));
    Ok(repo::list_audit(conn, &filter, pattern.as_deref(), before, limit).await?)
}

/// `text` as a `LIKE` pattern that matches it as it is: `%`, `_` and `\` are escaped.
fn like_pattern(text: &str) -> String {
    let mut pattern = String::with_capacity(text.len() + 2);
    pattern.push('%');
    for c in text.chars() {
        if matches!(c, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    pattern
}

/// Records something the server CLI did, where nobody is signed in: no actor, the source `cli`
/// (ADR-0034). Same transaction rule as [`audit`].
pub async fn audit_cli(conn: &mut PgConnection, entry: AuditEntry) -> Result<(), EventsError> {
    repo::insert_audit(conn, None, "cli", &entry).await?;
    Ok(())
}

/// Writes `event` to the outbox on the caller's connection, in the same transaction as the state
/// change it announces (ADR-0007). Draft saves never emit.
pub async fn emit(conn: &mut PgConnection, event: Event) -> Result<(), EventsError> {
    repo::insert_outbox(conn, &event).await?;
    Ok(())
}

/// Like [`audit`], but only when `actor` acts as an implicit course owner. Admins have no
/// membership, so everything they change inside a course is audited (ADR-0004); other actors
/// write nothing.
pub async fn audit_admin(
    conn: &mut PgConnection,
    actor: &Actor,
    entry: AuditEntry,
) -> Result<(), EventsError> {
    if implicit_course_owner(actor) {
        audit(conn, actor, entry).await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The outbox worker (ADR-0045)
// ---------------------------------------------------------------------------------------------

/// Rows claimed per round.
const BATCH: i64 = 20;
/// How long a claimed row belongs to the worker that claimed it.
const LEASE: Duration = Duration::from_secs(5 * 60);
/// Failed attempts after which a row becomes a dead letter.
const MAX_ATTEMPTS: i32 = 5;
/// How long the worker sleeps when there was nothing to do.
const POLL: Duration = Duration::from_secs(2);
/// The longest wait before a retry.
const MAX_BACKOFF: Duration = Duration::from_secs(60 * 60);
const MAX_ERROR_CHARS: usize = 500;

/// 10 s, 20 s, 40 s, ... capped at an hour.
fn backoff(attempts: i32) -> Duration {
    let exponent = u32::try_from(attempts.saturating_sub(1))
        .unwrap_or(0)
        .min(20);
    Duration::from_secs(10)
        .saturating_mul(2u32.saturating_pow(exponent))
        .min(MAX_BACKOFF)
}

/// Claims one batch and runs the handler of each row, outside any transaction. A row whose handler
/// succeeds, or that has no handler, is marked processed; a failure schedules a retry or, at the
/// last attempt, a dead letter. Returns how many rows it claimed.
pub async fn process_once(pool: &PgPool, handlers: &Handlers) -> Result<usize, EventsError> {
    let claimed = {
        let mut conn = pool.acquire().await?;
        repo::claim(&mut conn, BATCH, LEASE.as_secs_f64()).await?
    };
    let claimed_count = claimed.len();
    for (event, attempts) in claimed {
        let outcome = match handlers.get(&event.kind) {
            Some(handler) => handler(event.clone()).await,
            None => Ok(()),
        };
        let mut conn = pool.acquire().await?;
        match outcome {
            Ok(()) => repo::mark_processed(&mut conn, event.id).await?,
            Err(error) => {
                let give_up = attempts >= MAX_ATTEMPTS;
                let text: String = error.0.chars().take(MAX_ERROR_CHARS).collect();
                if give_up {
                    tracing::error!(event_id = %event.id, kind = %event.kind, attempts, error = %text, "event given up");
                } else {
                    tracing::warn!(event_id = %event.id, kind = %event.kind, attempts, error = %text, "event failed, will retry");
                }
                repo::mark_failed(
                    &mut conn,
                    event.id,
                    &text,
                    give_up,
                    backoff(attempts).as_secs_f64(),
                )
                .await?;
            }
        }
    }
    Ok(claimed_count)
}

/// Delivers the outbox until the process ends. ponytail: one row at a time and no graceful stop (a
/// handler cut off by a shutdown runs again after its lease); add a concurrency limit and a stop
/// signal when throughput or deploys need them.
pub async fn run_worker(pool: PgPool, handlers: Handlers) {
    loop {
        match process_once(&pool, &handlers).await {
            Ok(n) if n as i64 >= BATCH => continue,
            Ok(_) => {}
            Err(error) => tracing::error!(%error, "the outbox worker failed a round"),
        }
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use uuid::Uuid;

    use super::*;
    use crate::events::{HandlerError, HandlerFuture, OutboxEvent};
    use crate::test_support::db_test;

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
}
