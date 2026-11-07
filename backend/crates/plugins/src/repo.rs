//! SQL only. Every function takes the connection it runs on and never begins a transaction. This
//! crate reads and writes the schema `plugins` and nothing else.

use access::{CourseRole, CourseScope, Scope};
use chrono::{DateTime, Utc};
use common::Role;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::model::{Install, InstallStatus, ReportSubject, RouteSpec, UiSlot};
use crate::settings::SettingSpec;

/// The columns of `plugin_installs`, as the database holds them.
struct InstallRow {
    id: Uuid,
    plugin_id: String,
    version: String,
    base_url: String,
    subscribes: Vec<String>,
    requested_scopes: Vec<String>,
    scopes: Vec<String>,
    all_courses: bool,
    courses: Vec<Uuid>,
    ui_slots: serde_json::Value,
    settings: serde_json::Value,
    routes: serde_json::Value,
    secret_version: i32,
    status: InstallStatus,
    created_at: DateTime<Utc>,
}

/// The pages in `ui_slots`. They were checked when the plugin was installed; an entry that does not
/// read back is left out.
fn ui_of(value: &serde_json::Value) -> Vec<UiSlot> {
    let text = |entry: &serde_json::Value, key: &str| entry[key].as_str().map(str::to_owned);
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some(UiSlot {
                slot: text(entry, "slot")?,
                title: text(entry, "title")?,
                path: text(entry, "path")?,
            })
        })
        .collect()
}

/// The routes in `routes`. They were checked when the plugin was installed; an entry that does not
/// read back is left out, so it is a route nobody may call.
pub(crate) fn routes_of(value: &serde_json::Value) -> Vec<RouteSpec> {
    let text = |entry: &serde_json::Value, key: &str| entry[key].as_str().map(str::to_owned);
    let list = |entry: &serde_json::Value, key: &str| -> Vec<String> {
        entry[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()
    };
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some(RouteSpec {
                method: text(entry, "method")?,
                path: text(entry, "path")?,
                roles: list(entry, "roles")
                    .iter()
                    .filter_map(|r| r.parse::<Role>().ok())
                    .collect(),
                course_roles: list(entry, "course_roles")
                    .iter()
                    .filter_map(|r| r.parse::<CourseRole>().ok())
                    .collect(),
            })
        })
        .collect()
}

pub(crate) fn routes_json(routes: &[RouteSpec]) -> serde_json::Value {
    routes
        .iter()
        .map(|route| {
            serde_json::json!({
                "method": route.method,
                "path": route.path,
                "roles": route.roles.iter().map(|r| r.as_str()).collect::<Vec<_>>(),
                "course_roles": route.course_roles.iter().map(|r| r.as_str()).collect::<Vec<_>>(),
            })
        })
        .collect()
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
            courses: course_scope(row.all_courses, row.courses),
            ui: ui_of(&row.ui_slots),
            // Checked when the plugin was installed; one that does not read back declares nothing.
            settings: serde_json::from_value(row.settings).unwrap_or_default(),
            routes: routes_of(&row.routes),
            secret_version: row.secret_version,
            status: row.status,
            created_at: row.created_at,
        }
    }
}

pub(crate) fn course_scope(all_courses: bool, courses: Vec<Uuid>) -> CourseScope {
    if all_courses {
        CourseScope::All
    } else {
        CourseScope::Only(courses)
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
    pub courses: &'a CourseScope,
    pub ui_slots: serde_json::Value,
    pub routes: serde_json::Value,
    pub settings: &'a std::collections::BTreeMap<String, SettingSpec>,
    pub token_hash: &'a [u8],
}

pub async fn insert_install(
    conn: &mut PgConnection,
    new: NewInstall<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO plugins.plugin_installs
             (id, plugin_id, version, base_url, api, subscribes, requested_scopes, scopes,
              ui_slots, settings, token_hash, all_courses, routes)
         VALUES ($1, $2, $3, $4, '1', $5, $6, $7, $8, $9, $10, $11, $12)",
        new.id,
        new.plugin_id,
        new.version,
        new.base_url,
        new.subscribes,
        new.requested_scopes,
        new.scopes,
        new.ui_slots,
        serde_json::to_value(new.settings).unwrap_or_default(),
        new.token_hash,
        matches!(new.courses, CourseScope::All),
        new.routes
    )
    .execute(&mut *conn)
    .await?;
    set_courses(conn, new.id, new.courses).await
}

/// Makes the courses of the install exactly `courses`: `All` keeps no list, `Only` keeps these.
pub async fn set_courses(
    conn: &mut PgConnection,
    install_id: Uuid,
    courses: &CourseScope,
) -> Result<(), sqlx::Error> {
    let (all, list): (bool, &[Uuid]) = match courses {
        CourseScope::All => (true, &[]),
        CourseScope::Only(list) => (false, list),
    };
    sqlx::query!(
        "UPDATE plugins.plugin_installs SET all_courses = $2, updated_at = now() WHERE id = $1",
        install_id,
        all
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!(
        "DELETE FROM plugins.install_courses WHERE install_id = $1",
        install_id
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!(
        "INSERT INTO plugins.install_courses (install_id, course_id)
         SELECT $1, course_id FROM unnest($2::uuid[]) AS course_id",
        install_id,
        list
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
                  scopes AS "scopes!", all_courses,
                  ARRAY(SELECT course_id FROM plugins.install_courses
                        WHERE install_id = plugin_installs.id) AS "courses!",
                  ui_slots, settings, routes, secret_version,
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
                  scopes AS "scopes!", all_courses,
                  ARRAY(SELECT course_id FROM plugins.install_courses
                        WHERE install_id = plugin_installs.id) AS "courses!",
                  ui_slots, settings, routes, secret_version,
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
    pub all_courses: bool,
    pub courses: Vec<Uuid>,
}

pub async fn find_enabled_by_token(
    conn: &mut PgConnection,
    token_hash: &[u8],
) -> Result<Option<TokenOwner>, sqlx::Error> {
    sqlx::query_as!(
        TokenOwner,
        r#"SELECT id AS install_id, plugin_id, scopes AS "scopes!", all_courses,
                  ARRAY(SELECT course_id FROM plugins.install_courses
                        WHERE install_id = plugin_installs.id) AS "courses!"
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

/// The enabled installs that subscribed to `kind` and, when the event is about a course, may reach
/// it (ADR-0074).
pub async fn enabled_subscribers(
    conn: &mut PgConnection,
    kind: &str,
    course_id: Option<Uuid>,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT id FROM plugins.plugin_installs i
         WHERE status = 'enabled' AND $1 = ANY (subscribes)
           AND ($2::uuid IS NULL OR all_courses
                OR EXISTS (SELECT 1 FROM plugins.install_courses c
                           WHERE c.install_id = i.id AND c.course_id = $2))
         ORDER BY id",
        kind,
        course_id
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
    occurred_at: DateTime<Utc>,
) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "INSERT INTO plugins.deliveries (id, install_id, event_id, event_type, data, occurred_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (install_id, event_id) DO NOTHING",
        Uuid::now_v7(),
        install_id,
        event_id,
        event_type,
        data,
        occurred_at
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
    pub occurred_at: DateTime<Utc>,
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
               RETURNING d.id, d.install_id, d.event_id, d.event_type, d.data, d.occurred_at, d.attempts
           )
           SELECT c.id AS "id!", c.install_id AS "install_id!", c.event_id AS "event_id!",
                  c.event_type AS "event_type!", c.data AS "data!",
                  c.occurred_at AS "occurred_at!", c.attempts AS "attempts!",
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

/// One report per plugin, subject and key: a later one replaces the earlier.
pub async fn upsert_report(
    conn: &mut PgConnection,
    install_id: Uuid,
    subject: ReportSubject,
    key: &str,
    summary: &str,
    data: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    let (submission, assignment, course) = match subject {
        ReportSubject::Submission(id) => (Some(id), None, None),
        ReportSubject::Assignment(id) => (None, Some(id), None),
        ReportSubject::Course(id) => (None, None, Some(id)),
    };
    sqlx::query!(
        "INSERT INTO plugins.reports
             (install_id, submission_id, assignment_id, course_id, key, summary, data)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (install_id, subject_id, key)
         DO UPDATE SET summary = $6, data = $7, updated_at = now()",
        install_id,
        submission,
        assignment,
        course,
        key,
        summary,
        data
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Whether the plugin keeps a report under this key on the subject.
pub async fn report_exists(
    conn: &mut PgConnection,
    install_id: Uuid,
    subject: ReportSubject,
    key: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM plugins.reports
                          WHERE install_id = $1 AND subject_id = $2 AND key = $3) AS "found!""#,
        install_id,
        subject.id(),
        key
    )
    .fetch_one(conn)
    .await
}

/// How many reports the plugin keeps on the subject.
pub async fn count_reports(
    conn: &mut PgConnection,
    install_id: Uuid,
    subject: ReportSubject,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM plugins.reports
           WHERE install_id = $1 AND subject_id = $2"#,
        install_id,
        subject.id()
    )
    .fetch_one(conn)
    .await
}

/// Whether there was a report to delete.
pub async fn delete_report(
    conn: &mut PgConnection,
    install_id: Uuid,
    subject: ReportSubject,
    key: &str,
) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "DELETE FROM plugins.reports WHERE install_id = $1 AND subject_id = $2 AND key = $3",
        install_id,
        subject.id(),
        key
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected() == 1)
}

pub struct ReportRow {
    pub plugin_id: String,
    pub key: String,
    pub summary: String,
    pub data: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

/// The reports on a subject, by plugin and key. A disabled plugin's reports stay; they were written
/// while it was allowed to.
pub async fn reports_of(
    conn: &mut PgConnection,
    subject: ReportSubject,
) -> Result<Vec<ReportRow>, sqlx::Error> {
    sqlx::query_as!(
        ReportRow,
        "SELECT i.plugin_id, r.key, r.summary, r.data, r.updated_at
         FROM plugins.reports r JOIN plugins.plugin_installs i ON i.id = r.install_id
         WHERE r.subject_id = $1
         ORDER BY i.plugin_id, r.key",
        subject.id()
    )
    .fetch_all(conn)
    .await
}

// ---------------------------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------------------------

/// The nil uuid stands for "no course" in the unique index of `plugins.settings`.
const NO_COURSE: Uuid = Uuid::nil();

/// A stored value of one setting. For a secret it is the sealed string.
pub struct StoredSetting {
    pub key: String,
    pub value: serde_json::Value,
}

/// The values stored for the install in one scope: the whole institution (`course_id` is `None`) or
/// one course.
pub async fn settings_of(
    conn: &mut PgConnection,
    install_id: Uuid,
    course_id: Option<Uuid>,
) -> Result<Vec<StoredSetting>, sqlx::Error> {
    sqlx::query_as!(
        StoredSetting,
        "SELECT key, value FROM plugins.settings
         WHERE install_id = $1 AND COALESCE(course_id, $3) = COALESCE($2, $3)
         ORDER BY key",
        install_id,
        course_id,
        NO_COURSE
    )
    .fetch_all(conn)
    .await
}

pub async fn upsert_setting(
    conn: &mut PgConnection,
    install_id: Uuid,
    course_id: Option<Uuid>,
    key: &str,
    value: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO plugins.settings (install_id, course_id, key, value)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (install_id, key, (COALESCE(course_id, '00000000-0000-0000-0000-000000000000')))
         DO UPDATE SET value = $4, updated_at = now()",
        install_id,
        course_id,
        key,
        value
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Whether a value was there to delete.
pub async fn delete_setting(
    conn: &mut PgConnection,
    install_id: Uuid,
    course_id: Option<Uuid>,
    key: &str,
) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "DELETE FROM plugins.settings
         WHERE install_id = $1 AND key = $3 AND COALESCE(course_id, $4) = COALESCE($2, $4)",
        install_id,
        course_id,
        key,
        NO_COURSE
    )
    .execute(conn)
    .await?;
    Ok(done.rows_affected() == 1)
}
