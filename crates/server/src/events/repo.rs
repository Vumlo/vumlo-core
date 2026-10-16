//! SQL only. Every function takes the connection it runs on and never begins a transaction.

use sqlx::PgConnection;
use uuid::Uuid;

use super::model::AuditEntry;

pub async fn insert_audit(
    conn: &mut PgConnection,
    actor_id: Uuid,
    entry: &AuditEntry,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO events.audit_log (id, actor_id, action, resource_type, resource_id, meta)
         VALUES ($1, $2, $3, $4, $5, $6)",
        Uuid::now_v7(),
        actor_id,
        entry.action,
        entry.resource_type,
        entry.resource_id,
        entry.meta
    )
    .execute(conn)
    .await?;
    Ok(())
}
