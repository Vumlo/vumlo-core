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
mod tests;
