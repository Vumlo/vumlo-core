//! SQL only. Every function is one statement on a pooled connection: the record must be visible to a
//! concurrent retry at once, so it is not part of the transaction of the request it guards (ADR-0058).

use chrono::{DateTime, Utc};
use sqlx::PgConnection;

/// What a finished request left behind.
pub struct Stored {
    pub status: i16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

pub enum Claim {
    /// Nobody has used this key (or its record expired or was abandoned): the caller runs the request.
    Fresh,
    Replay(Stored),
    /// The key was used for another request.
    Mismatch,
    /// The first request with this key is still running.
    Running,
}

/// A request that has been running for this long is taken to be lost with its process (the request
/// timeout is shorter), and a retry may run it again.
const ABANDONED_AFTER_SECS: f64 = 120.0;

pub async fn claim(
    conn: &mut PgConnection,
    scope: &[u8],
    key: &str,
    request: &[u8],
) -> Result<Claim, sqlx::Error> {
    // The expired records of every key go on the way in, as `identity.oauth_states` does.
    sqlx::query!("DELETE FROM idempotency.keys WHERE expires_at <= now()")
        .execute(&mut *conn)
        .await?;
    // A record can vanish between the insert and the read (its owner released it): try again.
    for _ in 0..3 {
        let inserted = sqlx::query!(
            "INSERT INTO idempotency.keys (scope_hash, key, request_hash) VALUES ($1, $2, $3)
             ON CONFLICT (scope_hash, key) DO NOTHING",
            scope,
            key,
            request,
        )
        .execute(&mut *conn)
        .await?
        .rows_affected();
        if inserted == 1 {
            return Ok(Claim::Fresh);
        }
        let Some(row) = sqlx::query!(
            r#"SELECT request_hash, status, content_type, body, started_at
               FROM idempotency.keys WHERE scope_hash = $1 AND key = $2"#,
            scope,
            key,
        )
        .fetch_optional(&mut *conn)
        .await?
        else {
            continue;
        };
        if row.request_hash != request {
            return Ok(Claim::Mismatch);
        }
        if let (Some(status), Some(body)) = (row.status, row.body) {
            return Ok(Claim::Replay(Stored {
                status,
                content_type: row.content_type,
                body,
            }));
        }
        if abandoned(row.started_at) && take_over(conn, scope, key, row.started_at).await? {
            return Ok(Claim::Fresh);
        }
        return Ok(Claim::Running);
    }
    Ok(Claim::Running)
}

fn abandoned(started_at: DateTime<Utc>) -> bool {
    (Utc::now() - started_at).num_milliseconds() as f64 / 1000.0 > ABANDONED_AFTER_SECS
}

/// One retry wins the abandoned record: the update matches only the `started_at` that was read.
async fn take_over(
    conn: &mut PgConnection,
    scope: &[u8],
    key: &str,
    started_at: DateTime<Utc>,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query!(
        "UPDATE idempotency.keys SET started_at = now()
         WHERE scope_hash = $1 AND key = $2 AND status IS NULL AND started_at = $3",
        scope,
        key,
        started_at,
    )
    .execute(conn)
    .await?
    .rows_affected()
        == 1)
}

pub async fn complete(
    conn: &mut PgConnection,
    scope: &[u8],
    key: &str,
    stored: &Stored,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE idempotency.keys SET status = $3, content_type = $4, body = $5
         WHERE scope_hash = $1 AND key = $2 AND status IS NULL",
        scope,
        key,
        stored.status,
        stored.content_type,
        stored.body,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The request left no answer worth keeping (a server error, a throttle): the key is free again.
pub async fn release(conn: &mut PgConnection, scope: &[u8], key: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "DELETE FROM idempotency.keys WHERE scope_hash = $1 AND key = $2 AND status IS NULL",
        scope,
        key,
    )
    .execute(conn)
    .await?;
    Ok(())
}
