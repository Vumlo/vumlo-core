//! SQL only. Every function takes the connection it runs on and never begins a transaction.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::model::{Filter, JournalEntry, MessageState, Status};

// ---------------------------------------------------------------------------------------------
// Enqueue
// ---------------------------------------------------------------------------------------------

pub async fn origin_queued(conn: &mut PgConnection, origin_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM mail.messages WHERE origin_id = $1) AS "queued!""#,
        origin_id
    )
    .fetch_one(conn)
    .await
}

pub struct NewBatch<'a> {
    pub id: Uuid,
    pub plugin_id: Option<&'a str>,
    pub kind: &'a str,
    pub subject: &'a str,
    pub body: &'a str,
    pub link_path: Option<&'a str>,
    pub accepted: i32,
    pub skipped: serde_json::Value,
}

pub async fn insert_batch(
    conn: &mut PgConnection,
    batch: &NewBatch<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO mail.batches (id, plugin_id, kind, subject, body, link_path, accepted, skipped)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        batch.id,
        batch.plugin_id,
        batch.kind,
        batch.subject,
        batch.body,
        batch.link_path,
        batch.accepted,
        batch.skipped
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn delete_batch(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM mail.batches WHERE id = $1", id)
        .execute(conn)
        .await?;
    Ok(())
}

pub struct NewAccountMessage<'a> {
    pub id: Uuid,
    pub batch_id: Uuid,
    pub user_id: Uuid,
    pub kind: &'a str,
    pub to_address: &'a str,
    pub to_name: &'a str,
    pub origin_id: Uuid,
}

/// How many rows it inserted: 0 when `origin_id` was queued already.
pub async fn insert_account_message(
    conn: &mut PgConnection,
    message: &NewAccountMessage<'_>,
) -> Result<u64, sqlx::Error> {
    let done = sqlx::query!(
        "INSERT INTO mail.messages (id, batch_id, user_id, kind, to_address, to_name, origin_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (origin_id) DO NOTHING",
        message.id,
        message.batch_id,
        message.user_id,
        message.kind,
        message.to_address,
        message.to_name,
        message.origin_id
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected())
}

/// Serializes the calls of one plugin until the transaction ends, so the hourly count is exact.
pub async fn lock_plugin(conn: &mut PgConnection, plugin_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "SELECT pg_advisory_xact_lock(hashtextextended('vumlo-mail-plugin|' || $1, 0))",
        plugin_id
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// How many calls the plugin has made in the last `window_secs`, the age of the oldest of them in
/// seconds.
pub async fn calls_in_window(
    conn: &mut PgConnection,
    plugin_id: &str,
    window_secs: f64,
) -> Result<(i64, Option<f64>), sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT count(*) AS "calls!",
                  EXTRACT(EPOCH FROM (now() - min(created_at)))::float8 AS age
           FROM mail.batches
           WHERE plugin_id = $1 AND created_at > now() - make_interval(secs => $2)"#,
        plugin_id,
        window_secs
    )
    .fetch_one(conn)
    .await?;
    Ok((row.calls, row.age))
}

pub async fn opted_out(
    conn: &mut PgConnection,
    plugin_id: &str,
    kind: &str,
    users: &[Uuid],
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT user_id FROM mail.opt_outs
         WHERE plugin_id = $1 AND kind = $2 AND user_id = ANY($3)",
        plugin_id,
        kind,
        users
    )
    .fetch_all(conn)
    .await
}

pub struct Recent {
    pub user_id: Uuid,
    pub count: i64,
    /// Seconds since the oldest of them.
    pub oldest_age: f64,
}

/// Plugin messages each of `users` got in the last `window_secs`, for those who got any.
pub async fn recent_plugin_messages(
    conn: &mut PgConnection,
    users: &[Uuid],
    window_secs: f64,
) -> Result<Vec<Recent>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT user_id, count(*) AS "count!",
                  EXTRACT(EPOCH FROM (now() - min(created_at)))::float8 AS "age!"
           FROM mail.messages
           WHERE plugin_id IS NOT NULL AND user_id = ANY($1)
             AND created_at > now() - make_interval(secs => $2)
           GROUP BY user_id"#,
        users,
        window_secs
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Recent {
            user_id: row.user_id,
            count: row.count,
            oldest_age: row.age,
        })
        .collect())
}

/// Claims `(plugin, key, user)` for each of `users` that has not had it in `window_days`; returns
/// the ones it claimed.
pub async fn claim_dedupe(
    conn: &mut PgConnection,
    plugin_id: &str,
    key: &str,
    users: &[Uuid],
    window_days: i32,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"INSERT INTO mail.dedupe (plugin_id, dedupe_key, user_id)
           SELECT $1, $2, u FROM UNNEST($3::uuid[]) AS u
           ON CONFLICT (plugin_id, dedupe_key, user_id) DO UPDATE SET at = now()
             WHERE mail.dedupe.at < now() - make_interval(days => $4)
           RETURNING user_id"#,
        plugin_id,
        key,
        users,
        window_days
    )
    .fetch_all(conn)
    .await
}

pub struct NewMessages<'a> {
    pub batch_id: Uuid,
    pub plugin_id: &'a str,
    pub kind: &'a str,
    pub ids: &'a [Uuid],
    pub users: &'a [Uuid],
    pub addresses: &'a [String],
    pub names: &'a [String],
}

pub async fn insert_plugin_messages(
    conn: &mut PgConnection,
    messages: &NewMessages<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO mail.messages (id, batch_id, user_id, plugin_id, kind, to_address, to_name)
         SELECT m.id, $1, m.user_id, $2, $3, m.address, m.name
         FROM UNNEST($4::uuid[], $5::uuid[], $6::text[], $7::text[]) AS m(id, user_id, address, name)",
        messages.batch_id,
        messages.plugin_id,
        messages.kind,
        messages.ids,
        messages.users,
        messages.addresses,
        messages.names
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Cancels the plugin's pending messages and erases their addresses at once: they will never be
/// sent. Returns how many.
pub async fn cancel_pending(conn: &mut PgConnection, plugin_id: &str) -> Result<u64, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE mail.messages
         SET state = 'cancelled', finished_at = now(), erase_at = now(), last_error = NULL
         WHERE state = 'pending' AND plugin_id = $1",
        plugin_id
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected())
}

// ---------------------------------------------------------------------------------------------
// The worker
// ---------------------------------------------------------------------------------------------

pub struct Claimed {
    pub id: Uuid,
    pub user_id: Uuid,
    pub plugin_id: Option<String>,
    pub kind: String,
    pub to_address: Option<String>,
    pub to_name: Option<String>,
    pub attempts: i32,
    pub expired: bool,
    pub subject: Option<String>,
    pub body: Option<String>,
    pub link_path: Option<String>,
}

/// Claims up to `limit` due messages with a lease: `attempts` goes up and `run_after` moves
/// `lease_secs` forward, so a crashed worker's messages come back later (ADR-0084). Oldest first.
pub async fn claim(
    conn: &mut PgConnection,
    limit: i64,
    lease_secs: f64,
) -> Result<Vec<Claimed>, sqlx::Error> {
    let mut rows = sqlx::query!(
        r#"WITH due AS (
               SELECT id FROM mail.messages
               WHERE state = 'pending' AND run_after <= now()
               ORDER BY run_after, id LIMIT $1 FOR UPDATE SKIP LOCKED
           ), claimed AS (
               UPDATE mail.messages m
               SET attempts = m.attempts + 1, run_after = now() + make_interval(secs => $2)
               FROM due WHERE m.id = due.id
               RETURNING m.id, m.batch_id, m.user_id, m.plugin_id, m.kind, m.to_address, m.to_name,
                         m.attempts, m.expires_at <= now() AS expired, m.run_after
           )
           SELECT c.id AS "id!", c.user_id AS "user_id!", c.plugin_id, c.kind AS "kind!",
                  c.to_address, c.to_name, c.attempts AS "attempts!", c.expired AS "expired!",
                  b.subject, b.body, b.link_path
           FROM claimed c JOIN mail.batches b ON b.id = c.batch_id"#,
        limit,
        lease_secs
    )
    .fetch_all(conn)
    .await?;
    rows.sort_by_key(|row| row.id);
    Ok(rows
        .into_iter()
        .map(|row| Claimed {
            id: row.id,
            user_id: row.user_id,
            plugin_id: row.plugin_id,
            kind: row.kind,
            to_address: row.to_address,
            to_name: row.to_name,
            attempts: row.attempts,
            expired: row.expired,
            subject: row.subject,
            body: row.body,
            link_path: row.link_path,
        })
        .collect())
}

pub async fn is_opted_out(
    conn: &mut PgConnection,
    user_id: Uuid,
    plugin_id: &str,
    kind: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM mail.opt_outs
                          WHERE user_id = $1 AND plugin_id = $2 AND kind = $3) AS "out!""#,
        user_id,
        plugin_id,
        kind
    )
    .fetch_one(conn)
    .await
}

/// Ends a message: `state` is `sent`, `dead` or `cancelled`. Its address goes `erase_days` from now.
pub async fn finish(
    conn: &mut PgConnection,
    id: Uuid,
    state: MessageState,
    last_error: Option<&str>,
    erase_days: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE mail.messages
         SET state = $2, last_error = $3, finished_at = now(),
             erase_at = now() + make_interval(days => $4)
         WHERE id = $1",
        id,
        state.as_str(),
        last_error,
        erase_days
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// A failed attempt that will be tried again in `retry_secs`.
pub async fn schedule_retry(
    conn: &mut PgConnection,
    id: Uuid,
    last_error: &str,
    retry_secs: f64,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE mail.messages
         SET last_error = $2, run_after = now() + make_interval(secs => $3)
         WHERE id = $1",
        id,
        last_error,
        retry_secs
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn signing_key(conn: &mut PgConnection) -> Result<Option<Vec<u8>>, sqlx::Error> {
    sqlx::query_scalar!("SELECT key FROM mail.signing_key WHERE id")
        .fetch_optional(conn)
        .await
}

/// Stores `key` unless a key exists already (a second process may have made one first).
pub async fn insert_signing_key(conn: &mut PgConnection, key: &[u8]) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO mail.signing_key (key) VALUES ($1) ON CONFLICT (id) DO NOTHING",
        key
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn set_opt_out(
    conn: &mut PgConnection,
    user_id: Uuid,
    plugin_id: &str,
    kind: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO mail.opt_outs (user_id, plugin_id, kind) VALUES ($1, $2, $3)
         ON CONFLICT DO NOTHING",
        user_id,
        plugin_id,
        kind
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn clear_opt_out(
    conn: &mut PgConnection,
    user_id: Uuid,
    plugin_id: &str,
    kind: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "DELETE FROM mail.opt_outs WHERE user_id = $1 AND plugin_id = $2 AND kind = $3",
        user_id,
        plugin_id,
        kind
    )
    .execute(conn)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Retention
// ---------------------------------------------------------------------------------------------

/// Sets the address and the name of every ended message past its `erase_at` to NULL.
pub async fn erase_addresses(conn: &mut PgConnection) -> Result<u64, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE mail.messages SET to_address = NULL, to_name = NULL
         WHERE to_address IS NOT NULL AND erase_at IS NOT NULL AND erase_at <= now()"
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected())
}

/// Removes the subject, the text and the link of every batch that no longer holds an address.
pub async fn erase_batch_content(conn: &mut PgConnection) -> Result<u64, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE mail.batches b SET subject = NULL, body = NULL, link_path = NULL
         WHERE (b.subject IS NOT NULL OR b.body IS NOT NULL OR b.link_path IS NOT NULL)
           AND NOT EXISTS (SELECT 1 FROM mail.messages m
                           WHERE m.batch_id = b.id AND m.to_address IS NOT NULL)"
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected())
}

/// Deletes the batches older than `journal_days`, and their messages with them.
pub async fn delete_old_batches(
    conn: &mut PgConnection,
    journal_days: i32,
) -> Result<u64, sqlx::Error> {
    let done = sqlx::query!(
        "DELETE FROM mail.batches WHERE created_at < now() - make_interval(days => $1)",
        journal_days
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected())
}

pub async fn delete_old_dedupe(
    conn: &mut PgConnection,
    window_days: i32,
) -> Result<u64, sqlx::Error> {
    let done = sqlx::query!(
        "DELETE FROM mail.dedupe WHERE at < now() - make_interval(days => $1)",
        window_days
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected())
}

// ---------------------------------------------------------------------------------------------
// The journal (ADR-0087)
// ---------------------------------------------------------------------------------------------

pub async fn status(conn: &mut PgConnection) -> Result<Status, sqlx::Error> {
    let counts = sqlx::query!(
        r#"SELECT
             count(*) FILTER (WHERE state = 'pending') AS "pending!",
             count(*) FILTER (WHERE state = 'sent' AND finished_at > now() - interval '24 hours') AS "sent_24h!",
             count(*) FILTER (WHERE state = 'dead' AND finished_at > now() - interval '24 hours') AS "dead_24h!",
             count(*) FILTER (WHERE state = 'cancelled' AND finished_at > now() - interval '24 hours') AS "cancelled_24h!",
             count(*) FILTER (WHERE state = 'dead') AS "dead_now!",
             min(created_at) FILTER (WHERE state = 'pending') AS oldest_pending
           FROM mail.messages"#
    )
    .fetch_one(&mut *conn)
    .await?;
    let dead_by = sqlx::query!(
        r#"SELECT plugin_id, kind, count(*) AS "count!"
           FROM mail.messages WHERE state = 'dead'
           GROUP BY plugin_id, kind ORDER BY count(*) DESC, plugin_id, kind"#
    )
    .fetch_all(conn)
    .await?;
    Ok(Status {
        pending: counts.pending,
        sent_24h: counts.sent_24h,
        dead_24h: counts.dead_24h,
        cancelled_24h: counts.cancelled_24h,
        dead_now: counts.dead_now,
        oldest_pending: counts.oldest_pending,
        dead_by: dead_by
            .into_iter()
            .map(|row| (row.plugin_id, row.kind, row.count))
            .collect(),
    })
}

#[derive(sqlx::FromRow)]
struct JournalRow {
    id: Uuid,
    created_at: DateTime<Utc>,
    state: MessageState,
    attempts: i32,
    plugin_id: Option<String>,
    kind: String,
    user_id: Uuid,
    subject: Option<String>,
    last_error: Option<String>,
}

/// Newest first. The filters are bound parameters.
pub async fn list(
    conn: &mut PgConnection,
    filter: &Filter,
    limit: i64,
) -> Result<Vec<JournalEntry>, sqlx::Error> {
    let mut query = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT m.id, m.created_at, m.state, m.attempts, m.plugin_id, m.kind, m.user_id,
                b.subject, m.last_error
         FROM mail.messages m JOIN mail.batches b ON b.id = m.batch_id WHERE true",
    );
    if let Some(state) = filter.state {
        query.push(" AND m.state = ").push_bind(state.as_str());
    }
    if let Some(plugin) = &filter.plugin_id {
        query.push(" AND m.plugin_id = ").push_bind(plugin);
    }
    if let Some(kind) = &filter.kind {
        query.push(" AND m.kind = ").push_bind(kind);
    }
    if let Some(user) = filter.user_id {
        query.push(" AND m.user_id = ").push_bind(user);
    }
    if let Some(hours) = filter.hours {
        query
            .push(" AND m.created_at > now() - make_interval(hours => ")
            .push_bind(i32::try_from(hours).unwrap_or(i32::MAX))
            .push(")");
    }
    query.push(" ORDER BY m.id DESC LIMIT ").push_bind(limit);
    let rows: Vec<JournalRow> = query.build_query_as().fetch_all(conn).await?;
    Ok(rows
        .into_iter()
        .map(|row| JournalEntry {
            id: row.id,
            created_at: row.created_at,
            state: row.state,
            attempts: row.attempts,
            plugin_id: row.plugin_id,
            kind: row.kind,
            user_id: row.user_id,
            subject: row.subject,
            last_error: row.last_error,
        })
        .collect())
}

/// Dead messages in scope that still hold an address, and those that no longer do.
pub async fn dead_in_scope(
    conn: &mut PgConnection,
    id: Option<Uuid>,
    plugin_id: Option<&str>,
) -> Result<(Vec<Uuid>, Vec<Uuid>), sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT id, (to_address IS NOT NULL) AS "has_address!"
           FROM mail.messages
           WHERE state = 'dead'
             AND ($1::uuid IS NULL OR id = $1)
             AND ($2::text IS NULL OR plugin_id = $2)
           ORDER BY id"#,
        id,
        plugin_id
    )
    .fetch_all(conn)
    .await?;
    let (live, erased): (Vec<_>, Vec<_>) = rows.into_iter().partition(|row| row.has_address);
    Ok((
        live.into_iter().map(|row| row.id).collect(),
        erased.into_iter().map(|row| row.id).collect(),
    ))
}

/// Puts dead messages back: `pending`, no attempts, due now, 24 hours to live.
pub async fn requeue(conn: &mut PgConnection, ids: &[Uuid]) -> Result<u64, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE mail.messages
         SET state = 'pending', attempts = 0, run_after = now(),
             expires_at = now() + interval '24 hours',
             finished_at = NULL, erase_at = NULL, last_error = NULL
         WHERE state = 'dead' AND to_address IS NOT NULL AND id = ANY($1)",
        ids
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected())
}
