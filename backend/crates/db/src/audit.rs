//! The audit row of a command run by an operator (ADR-0034): no actor, source `cli`.

use sqlx::PgConnection;
use uuid::Uuid;

/// Writes one `events.audit_log` row with source `cli`, on the caller's transaction. `meta` holds
/// ids, counts and names of things only: never a password, a token or an email address.
pub async fn insert_cli_audit(
    conn: &mut PgConnection,
    action: &str,
    resource_type: &str,
    resource_id: Uuid,
    meta: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO events.audit_log (id, actor_id, source, action, resource_type, resource_id, meta)
         VALUES ($1, NULL, 'cli', $2, $3, $4, $5)",
        Uuid::now_v7(),
        action,
        resource_type,
        resource_id,
        meta
    )
    .execute(conn)
    .await?;
    Ok(())
}
