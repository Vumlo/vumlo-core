//! What a plugin says about a submitted attempt, an assignment or a course, and what staff read of
//! it (ADR-0077). A plugin writes a report under a key of its own choosing and writes it again to
//! replace it; the key is in the path.

use access::{Actor, ActorSource, PluginCaller, PluginSource};
use api_types::{PluginReport, PluginReports, ProblemDetails, PutReportRequest};
use axum::Json;
use axum::extract::{FromRef, Path, State};
use axum::http::StatusCode;
use common::{ApiError, ValidJson};
use learning::plugin_read;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::db;
use crate::model::{ReportSubject, is_report_key};
use crate::repo::ReportRow;
use crate::service;
use crate::state::PluginsState;

/// The calls of a plugin: it writes and removes its reports.
pub fn plugin_router<S>() -> OpenApiRouter<S>
where
    S: PluginSource + Clone + 'static,
    PluginsState: FromRef<S>,
{
    OpenApiRouter::new()
        .routes(routes!(put_submission_report, delete_submission_report))
        .routes(routes!(put_assignment_report, delete_assignment_report))
        .routes(routes!(put_course_report, delete_course_report))
}

/// The calls of staff: they read what the plugins reported.
pub fn staff_router<S>() -> OpenApiRouter<S>
where
    S: ActorSource + Clone + 'static,
    PluginsState: FromRef<S>,
{
    OpenApiRouter::new()
        .routes(routes!(list_submission_reports))
        .routes(routes!(list_assignment_reports))
        .routes(routes!(list_course_reports))
}

fn key_of(key: &str) -> Result<(), ApiError> {
    if is_report_key(key) {
        Ok(())
    } else {
        Err(ApiError::invalid_field(
            "key",
            "must be lowercase letters, digits, `_` and `-`, start with a letter, and have at most 40 characters",
        ))
    }
}

async fn put(
    state: &PluginsState,
    caller: &PluginCaller,
    subject: ReportSubject,
    key: &str,
    body: &PutReportRequest,
) -> Result<StatusCode, ApiError> {
    key_of(key)?;
    let mut tx = state.db.begin().await.map_err(db)?;
    service::put_report(&mut tx, caller, subject, key, &body.summary, &body.data).await?;
    tx.commit().await.map_err(db)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete(
    state: &PluginsState,
    caller: &PluginCaller,
    subject: ReportSubject,
    key: &str,
) -> Result<StatusCode, ApiError> {
    key_of(key)?;
    let mut tx = state.db.begin().await.map_err(db)?;
    service::delete_report(&mut tx, caller, subject, key).await?;
    tx.commit().await.map_err(db)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The plugin's report on a submitted attempt under `key`, replacing its earlier one under that key.
/// The scope `reports:write`. Sending the same report twice leaves one. A plugin keeps at most 20
/// keys on a subject.
#[utoipa::path(
    put,
    path = "/plugin-api/submissions/{id}/reports/{key}",
    tag = "plugin-api",
    security(("plugin" = [])),
    params(
        ("id" = Uuid, Path, description = "The `submission_id` of the event"),
        ("key" = String, Path, description = "The name of the report: lowercase letters, digits, `_`, `-`"),
    ),
    request_body = PutReportRequest,
    responses(
        (status = 204, description = "The report is stored"),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `reports:write`, or not the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such submitted attempt", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "The key, the summary or the data is not valid, or the plugin keeps the most reports it may",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn put_submission_report(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    Path((id, key)): Path<(Uuid, String)>,
    ValidJson(body): ValidJson<PutReportRequest>,
) -> Result<StatusCode, ApiError> {
    {
        let mut conn = state.db.acquire().await.map_err(db)?;
        plugin_read::reportable_submission(&mut conn, &caller, id).await?;
    }
    put(&state, &caller, ReportSubject::Submission(id), &key, &body).await
}

/// Removes the plugin's report on a submitted attempt under `key`. The scope `reports:write`.
#[utoipa::path(
    delete,
    path = "/plugin-api/submissions/{id}/reports/{key}",
    tag = "plugin-api",
    security(("plugin" = [])),
    params(
        ("id" = Uuid, Path, description = "The `submission_id` of the event"),
        ("key" = String, Path, description = "The name of the report"),
    ),
    responses(
        (status = 204, description = "The report is gone"),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `reports:write`, or not the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such submitted attempt, or no report under that key",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "The key is not valid", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn delete_submission_report(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    Path((id, key)): Path<(Uuid, String)>,
) -> Result<StatusCode, ApiError> {
    {
        let mut conn = state.db.acquire().await.map_err(db)?;
        plugin_read::reportable_submission(&mut conn, &caller, id).await?;
    }
    delete(&state, &caller, ReportSubject::Submission(id), &key).await
}

/// The plugin's report on an assignment under `key`, replacing its earlier one under that key. The
/// scope `reports:write`.
#[utoipa::path(
    put,
    path = "/plugin-api/assignments/{id}/reports/{key}",
    tag = "plugin-api",
    security(("plugin" = [])),
    params(
        ("id" = Uuid, Path, description = "An assignment"),
        ("key" = String, Path, description = "The name of the report: lowercase letters, digits, `_`, `-`"),
    ),
    request_body = PutReportRequest,
    responses(
        (status = 204, description = "The report is stored"),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `reports:write`, or not the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such assignment", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "The key, the summary or the data is not valid, or the plugin keeps the most reports it may",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn put_assignment_report(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    Path((id, key)): Path<(Uuid, String)>,
    ValidJson(body): ValidJson<PutReportRequest>,
) -> Result<StatusCode, ApiError> {
    {
        let mut conn = state.db.acquire().await.map_err(db)?;
        plugin_read::reportable_assignment(&mut conn, &caller, id).await?;
    }
    put(&state, &caller, ReportSubject::Assignment(id), &key, &body).await
}

/// Removes the plugin's report on an assignment under `key`. The scope `reports:write`.
#[utoipa::path(
    delete,
    path = "/plugin-api/assignments/{id}/reports/{key}",
    tag = "plugin-api",
    security(("plugin" = [])),
    params(
        ("id" = Uuid, Path, description = "An assignment"),
        ("key" = String, Path, description = "The name of the report"),
    ),
    responses(
        (status = 204, description = "The report is gone"),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `reports:write`, or not the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such assignment, or no report under that key",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "The key is not valid", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn delete_assignment_report(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    Path((id, key)): Path<(Uuid, String)>,
) -> Result<StatusCode, ApiError> {
    {
        let mut conn = state.db.acquire().await.map_err(db)?;
        plugin_read::reportable_assignment(&mut conn, &caller, id).await?;
    }
    delete(&state, &caller, ReportSubject::Assignment(id), &key).await
}

/// The plugin's report on a course under `key`, replacing its earlier one under that key. The scope
/// `reports:write`.
#[utoipa::path(
    put,
    path = "/plugin-api/courses/{id}/reports/{key}",
    tag = "plugin-api",
    security(("plugin" = [])),
    params(
        ("id" = Uuid, Path, description = "A course"),
        ("key" = String, Path, description = "The name of the report: lowercase letters, digits, `_`, `-`"),
    ),
    request_body = PutReportRequest,
    responses(
        (status = 204, description = "The report is stored"),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `reports:write`, or not the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "The key, the summary or the data is not valid, or the plugin keeps the most reports it may",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn put_course_report(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    Path((id, key)): Path<(Uuid, String)>,
    ValidJson(body): ValidJson<PutReportRequest>,
) -> Result<StatusCode, ApiError> {
    {
        let mut conn = state.db.acquire().await.map_err(db)?;
        plugin_read::reportable_course(&mut conn, &caller, id).await?;
    }
    put(&state, &caller, ReportSubject::Course(id), &key, &body).await
}

/// Removes the plugin's report on a course under `key`. The scope `reports:write`.
#[utoipa::path(
    delete,
    path = "/plugin-api/courses/{id}/reports/{key}",
    tag = "plugin-api",
    security(("plugin" = [])),
    params(
        ("id" = Uuid, Path, description = "A course"),
        ("key" = String, Path, description = "The name of the report"),
    ),
    responses(
        (status = 204, description = "The report is gone"),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `reports:write`, or not the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or no report under that key",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "The key is not valid", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn delete_course_report(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    Path((id, key)): Path<(Uuid, String)>,
) -> Result<StatusCode, ApiError> {
    {
        let mut conn = state.db.acquire().await.map_err(db)?;
        plugin_read::reportable_course(&mut conn, &caller, id).await?;
    }
    delete(&state, &caller, ReportSubject::Course(id), &key).await
}

fn api_reports(rows: Vec<ReportRow>) -> Json<PluginReports> {
    Json(PluginReports {
        items: rows
            .into_iter()
            .map(|row| PluginReport {
                plugin_id: row.plugin_id,
                key: row.key,
                summary: row.summary,
                data: row.data,
                updated_at: row.updated_at,
            })
            .collect(),
    })
}

/// What the plugins reported about a submitted attempt, by plugin and key. Staff and admins who may
/// see the attempt.
#[utoipa::path(
    get,
    path = "/submissions/{id}/plugin-reports",
    tag = "plugin-api",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "A submitted attempt")),
    responses(
        (status = 200, description = "The reports, by plugin and key", body = PluginReports),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the assignment but not its submissions",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such attempt, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn list_submission_reports(
    actor: Actor,
    State(state): State<PluginsState>,
    Path(id): Path<Uuid>,
) -> Result<Json<PluginReports>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(db)?;
    Ok(api_reports(service::reports(&mut conn, &actor, id).await?))
}

/// What the plugins reported about an assignment, by plugin and key. Staff and admins of its course.
#[utoipa::path(
    get,
    path = "/assignments/{id}/plugin-reports",
    tag = "plugin-api",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "An assignment")),
    responses(
        (status = 200, description = "The reports, by plugin and key", body = PluginReports),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the assignment but not its submissions",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such assignment, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn list_assignment_reports(
    actor: Actor,
    State(state): State<PluginsState>,
    Path(id): Path<Uuid>,
) -> Result<Json<PluginReports>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(db)?;
    Ok(api_reports(
        service::assignment_reports(&mut conn, &actor, id).await?,
    ))
}

/// What the plugins reported about a course, by plugin and key. Staff and admins of the course.
#[utoipa::path(
    get,
    path = "/courses/{id}/plugin-reports",
    tag = "plugin-api",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "A course")),
    responses(
        (status = 200, description = "The reports, by plugin and key", body = PluginReports),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the course but not its submissions",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn list_course_reports(
    actor: Actor,
    State(state): State<PluginsState>,
    Path(id): Path<Uuid>,
) -> Result<Json<PluginReports>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(db)?;
    Ok(api_reports(
        service::course_reports(&mut conn, &actor, id).await?,
    ))
}
