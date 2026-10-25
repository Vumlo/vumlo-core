//! SQL only. Every function takes the connection it runs on and never begins a transaction.

use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{AuditEntry, Event, OutboxEvent};

/// `actor_id` is `None` exactly when `source` is `cli` (a check of the table).
pub async fn insert_audit(
    conn: &mut PgConnection,
    actor_id: Option<Uuid>,
    source: &str,
    entry: &AuditEntry,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO events.audit_log (id, actor_id, source, action, resource_type, resource_id, meta)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        Uuid::now_v7(),
        actor_id,
        source,
        entry.action,
        entry.resource_type,
        entry.resource_id,
        entry.meta
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn insert_outbox(conn: &mut PgConnection, event: &Event) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO events.outbox (id, type, version, payload) VALUES ($1, $2, $3, $4)",
        Uuid::now_v7(),
        event.kind(),
        event.version(),
        event.payload()
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Claims up to `limit` due rows with a lease: `attempts` goes up and `run_after` moves
/// `lease_secs` forward, so a crashed worker's rows come back later (ADR-0045). Oldest first.
pub async fn claim(
    conn: &mut PgConnection,
    limit: i64,
    lease_secs: f64,
) -> Result<Vec<(OutboxEvent, i32)>, sqlx::Error> {
    let mut rows = sqlx::query!(
        r#"UPDATE events.outbox
           SET attempts = attempts + 1, run_after = now() + make_interval(secs => $2)
           WHERE id IN (
               SELECT id FROM events.outbox
               WHERE processed_at IS NULL AND failed_at IS NULL AND run_after <= now()
               ORDER BY id LIMIT $1 FOR UPDATE SKIP LOCKED)
           RETURNING id, type AS kind, version, payload, attempts"#,
        limit,
        lease_secs
    )
    .fetch_all(conn)
    .await?;
    rows.sort_by_key(|row| row.id);
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                OutboxEvent {
                    id: row.id,
                    kind: row.kind,
                    version: row.version,
                    payload: row.payload,
                },
                row.attempts,
            )
        })
        .collect())
}

pub async fn mark_processed(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE events.outbox SET processed_at = now(), last_error = NULL WHERE id = $1",
        id
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Records a failed attempt. `give_up` turns the row into a dead letter; otherwise it is due again
/// in `retry_secs`.
pub async fn mark_failed(
    conn: &mut PgConnection,
    id: Uuid,
    error: &str,
    give_up: bool,
    retry_secs: f64,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE events.outbox
         SET last_error = $2,
             failed_at = CASE WHEN $3 THEN now() END,
             run_after = now() + make_interval(secs => $4)
         WHERE id = $1",
        id,
        error,
        give_up,
        retry_secs
    )
    .execute(conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
