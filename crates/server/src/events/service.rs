use sqlx::PgConnection;

use super::model::{AuditEntry, Event};
use super::repo;
use crate::actor::Actor;
use crate::policy::implicit_course_owner;

#[derive(Debug, thiserror::Error)]
pub enum EventsError {
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
