use sqlx::PgConnection;

use super::model::AuditEntry;
use super::repo;
use crate::actor::Actor;

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
    repo::insert_audit(conn, actor.user_id(), &entry).await?;
    Ok(())
}
