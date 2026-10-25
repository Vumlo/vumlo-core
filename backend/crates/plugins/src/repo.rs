//! SQL only. Every function takes the connection it runs on and never begins a transaction. This
//! crate reads and writes the schema `plugins` and nothing else.

use access::Scope;
use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::model::{Install, InstallStatus};

/// The columns of `plugin_installs`, as the database holds them.
struct InstallRow {
    id: Uuid,
    plugin_id: String,
    version: String,
    base_url: String,
    subscribes: Vec<String>,
    requested_scopes: Vec<String>,
    scopes: Vec<String>,
    secret_version: i32,
    status: InstallStatus,
    created_at: DateTime<Utc>,
}

/// The scopes in the database were checked when they were written; one that does not parse (a
/// scope a later release removed) grants nothing.
pub(crate) fn scopes_of(names: &[String]) -> Vec<Scope> {
    names.iter().filter_map(|name| name.parse().ok()).collect()
}

impl From<InstallRow> for Install {
    fn from(row: InstallRow) -> Self {
        Self {
            id: row.id,
            plugin_id: row.plugin_id,
            version: row.version,
            base_url: row.base_url,
            subscribes: row.subscribes,
            requested_scopes: scopes_of(&row.requested_scopes),
            scopes: scopes_of(&row.scopes),
            secret_version: row.secret_version,
            status: row.status,
            created_at: row.created_at,
        }
    }
}

pub fn scope_names(scopes: &[Scope]) -> Vec<String> {
    scopes
        .iter()
        .map(|scope| scope.as_str().to_owned())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Installs
// ---------------------------------------------------------------------------------------------

pub struct NewInstall<'a> {
    pub id: Uuid,
    pub plugin_id: &'a str,
    pub version: &'a str,
    pub base_url: &'a str,
    pub subscribes: &'a [String],
    pub requested_scopes: &'a [String],
    pub scopes: &'a [String],
    pub ui_slots: serde_json::Value,
    pub token_hash: &'a [u8],
}

pub async fn insert_install(
    conn: &mut PgConnection,
    new: NewInstall<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO plugins.plugin_installs
             (id, plugin_id, version, base_url, api, subscribes, requested_scopes, scopes,
              ui_slots, token_hash)
         VALUES ($1, $2, $3, $4, '1', $5, $6, $7, $8, $9)",
        new.id,
        new.plugin_id,
        new.version,
        new.base_url,
        new.subscribes,
        new.requested_scopes,
        new.scopes,
        new.ui_slots,
        new.token_hash
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn find_install(
    conn: &mut PgConnection,
    plugin_id: &str,
) -> Result<Option<Install>, sqlx::Error> {
    let row = sqlx::query_as!(
        InstallRow,
        r#"SELECT id, plugin_id, version, base_url,
                  subscribes AS "subscribes!", requested_scopes AS "requested_scopes!",
                  scopes AS "scopes!", secret_version,
                  status AS "status: InstallStatus", created_at
           FROM plugins.plugin_installs WHERE plugin_id = $1"#,
        plugin_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(Install::from))
}

pub async fn list_installs(conn: &mut PgConnection) -> Result<Vec<Install>, sqlx::Error> {
    let rows = sqlx::query_as!(
        InstallRow,
        r#"SELECT id, plugin_id, version, base_url,
                  subscribes AS "subscribes!", requested_scopes AS "requested_scopes!",
                  scopes AS "scopes!", secret_version,
                  status AS "status: InstallStatus", created_at
           FROM plugins.plugin_installs ORDER BY plugin_id"#
    )
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(Install::from).collect())
}

/// (install id, pending deliveries, dead letters) for every install that has any.
pub async fn delivery_counts(
    conn: &mut PgConnection,
) -> Result<Vec<(Uuid, i64, i64)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT install_id,
                  count(*) FILTER (WHERE delivered_at IS NULL AND failed_at IS NULL) AS "pending!",
                  count(*) FILTER (WHERE failed_at IS NOT NULL) AS "failed!"
           FROM plugins.deliveries GROUP BY install_id"#
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.install_id, row.pending, row.failed))
        .collect())
}

pub async fn set_scopes(
    conn: &mut PgConnection,
    id: Uuid,
    scopes: &[String],
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE plugins.plugin_installs SET scopes = $2, updated_at = now() WHERE id = $1",
        id,
        scopes
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn set_status(
    conn: &mut PgConnection,
    id: Uuid,
    status: InstallStatus,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE plugins.plugin_installs SET status = $2, updated_at = now() WHERE id = $1",
        id,
        status.as_str()
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Removes the install; its deliveries and reports go with it (the foreign keys cascade).
pub async fn delete_install(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM plugins.plugin_installs WHERE id = $1", id)
        .execute(conn)
        .await?;
    Ok(())
}

/// Who a token belongs to: only an enabled install has one that works.
pub struct TokenOwner {
    pub install_id: Uuid,
    pub plugin_id: String,
    pub scopes: Vec<String>,
}

pub async fn find_enabled_by_token(
    conn: &mut PgConnection,
    token_hash: &[u8],
) -> Result<Option<TokenOwner>, sqlx::Error> {
    sqlx::query_as!(
        TokenOwner,
        r#"SELECT id AS install_id, plugin_id, scopes AS "scopes!"
           FROM plugins.plugin_installs
           WHERE token_hash = $1 AND status = 'enabled'"#,
        token_hash
    )
    .fetch_optional(conn)
    .await
}

// ---------------------------------------------------------------------------------------------
// Deliveries
// ---------------------------------------------------------------------------------------------

/// The enabled installs that subscribed to `kind`.
pub async fn enabled_subscribers(
    conn: &mut PgConnection,
    kind: &str,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT id FROM plugins.plugin_installs
         WHERE status = 'enabled' AND $1 = ANY (subscribes)
         ORDER BY id",
        kind
    )
    .fetch_all(conn)
    .await
}

/// Owes `install_id` this event. A second insert of the same event for the same install changes
/// nothing, so a fan-out that runs twice is harmless.
pub async fn insert_delivery(
    conn: &mut PgConnection,
    install_id: Uuid,
    event_id: Uuid,
    event_type: &str,
    data: &serde_json::Value,
) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "INSERT INTO plugins.deliveries (id, install_id, event_id, event_type, data)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (install_id, event_id) DO NOTHING",
        Uuid::now_v7(),
        install_id,
        event_id,
        event_type,
        data
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// A delivery that is due, with where it goes.
pub struct Claimed {
    pub id: Uuid,
    pub install_id: Uuid,
    pub event_id: Uuid,
    pub event_type: String,
    pub data: serde_json::Value,
    pub attempts: i32,
    pub base_url: String,
    pub secret_version: i32,
}

/// Takes up to `limit` due deliveries of enabled installs, oldest first. The claim counts the
/// attempt and moves `run_after` forward by the lease, so a delivery whose process died comes back
/// when the lease runs out (the outbox does the same, ADR-0045). Rows are skipped, not waited
/// for, when another worker holds them.
pub async fn claim(
    conn: &mut PgConnection,
    limit: i64,
    lease_secs: f64,
) -> Result<Vec<Claimed>, sqlx::Error> {
    sqlx::query_as!(
        Claimed,
        r#"WITH due AS (
               SELECT d.id FROM plugins.deliveries d
               JOIN plugins.plugin_installs i ON i.id = d.install_id
               WHERE d.delivered_at IS NULL AND d.failed_at IS NULL
                 AND d.run_after <= now() AND i.status = 'enabled'
               ORDER BY d.id
               LIMIT $1
               FOR UPDATE OF d SKIP LOCKED
           ), claimed AS (
               UPDATE plugins.deliveries d
               SET attempts = d.attempts + 1, run_after = now() + make_interval(secs => $2)
               FROM due WHERE d.id = due.id
               RETURNING d.id, d.install_id, d.event_id, d.event_type, d.data, d.attempts
           )
           SELECT c.id AS "id!", c.install_id AS "install_id!", c.event_id AS "event_id!",
                  c.event_type AS "event_type!", c.data AS "data!", c.attempts AS "attempts!",
                  i.base_url, i.secret_version
           FROM claimed c JOIN plugins.plugin_installs i ON i.id = c.install_id
           ORDER BY c.id"#,
        limit,
        lease_secs
    )
    .fetch_all(conn)
    .await
}

pub async fn mark_delivered(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE plugins.deliveries SET delivered_at = now(), last_error = NULL WHERE id = $1",
        id
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// A failed attempt: retry after `retry_secs`, or, when `give_up`, a dead letter.
pub async fn mark_failed(
    conn: &mut PgConnection,
    id: Uuid,
    error: &str,
    give_up: bool,
    retry_secs: f64,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE plugins.deliveries
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

// ---------------------------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------------------------

/// One report per plugin and attempt: a later one replaces the earlier.
pub async fn upsert_report(
    conn: &mut PgConnection,
    install_id: Uuid,
    submission_id: Uuid,
    summary: &str,
    data: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO plugins.reports (install_id, submission_id, summary, data)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (install_id, submission_id)
         DO UPDATE SET summary = $3, data = $4, updated_at = now()",
        install_id,
        submission_id,
        summary,
        data
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub struct ReportRow {
    pub plugin_id: String,
    pub summary: String,
    pub data: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

/// The reports on an attempt, by plugin. A disabled plugin's reports stay; they were written
/// while it was allowed to.
pub async fn reports_of(
    conn: &mut PgConnection,
    submission_id: Uuid,
) -> Result<Vec<ReportRow>, sqlx::Error> {
    sqlx::query_as!(
        ReportRow,
        "SELECT i.plugin_id, r.summary, r.data, r.updated_at
         FROM plugins.reports r JOIN plugins.plugin_installs i ON i.id = r.install_id
         WHERE r.submission_id = $1
         ORDER BY i.plugin_id",
        submission_id
    )
    .fetch_all(conn)
    .await
}
