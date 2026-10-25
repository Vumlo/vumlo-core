//! HTTP edge of the plugin API (ADR-0066): extractors, DTO mapping, OpenAPI annotations. A plugin
//! calls `/plugin-api/*` with its token; every handler checks a scope in the service it calls.
//! Writes open the transaction, call exactly one writing service function and commit; the reads
//! that decide whether the write may go ahead run before it.

use access::{Actor, ActorSource, PluginCaller, PluginSource};
use api_types::{PluginReport, PluginReports, PluginSubmission, ProblemDetails, PutReportRequest};
use axum::Json;
use axum::extract::{FromRef, Path, State};
use axum::http::StatusCode;
use common::{ApiError, ValidJson};
use learning::assessment;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::service;
use crate::state::PluginsState;

pub fn router<S>() -> OpenApiRouter<S>
where
    S: PluginSource + ActorSource + Clone + 'static,
    PluginsState: FromRef<S>,
{
    OpenApiRouter::new()
        .routes(routes!(get_submission))
        .routes(routes!(put_report))
        .routes(routes!(list_reports))
}

fn db(error: sqlx::Error) -> ApiError {
    ApiError::from_db("plugins", &error)
}

/// A submitted attempt: which work, whose, and what it says. The scope `submissions:read`. Never a
/// draft, a score or a grade.
#[utoipa::path(
    get,
    path = "/plugin-api/submissions/{id}",
    tag = "plugin-api",
    security(("plugin" = [])),
    params(("id" = Uuid, Path, description = "The `submission_id` of the event")),
    responses(
        (status = 200, description = "The submitted attempt", body = PluginSubmission),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `submissions:read`",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such submitted attempt", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn get_submission(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    Path(id): Path<Uuid>,
) -> Result<Json<PluginSubmission>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(db)?;
    let work = assessment::submission_for_plugin(&mut conn, &caller, id).await?;
    Ok(Json(PluginSubmission {
        id: work.id,
        assignment_id: work.assignment_id,
        student_id: work.student_id,
        attempt: work.attempt,
        content: work.content,
        submitted_at: work.submitted_at,
    }))
}

/// The plugin's report on a submitted attempt, replacing its earlier one. The scope
/// `reports:write`. Sending the same report twice leaves one.
#[utoipa::path(
    put,
    path = "/plugin-api/submissions/{id}/report",
    tag = "plugin-api",
    security(("plugin" = [])),
    params(("id" = Uuid, Path, description = "The `submission_id` of the event")),
    request_body = PutReportRequest,
    responses(
        (status = 204, description = "The report is stored"),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `reports:write`",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such submitted attempt", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "The summary or the data is not valid", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn put_report(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    Path(id): Path<Uuid>,
    ValidJson(body): ValidJson<PutReportRequest>,
) -> Result<StatusCode, ApiError> {
    {
        let mut conn = state.db.acquire().await.map_err(db)?;
        assessment::ensure_reportable(&mut conn, &caller, id).await?;
    }
    let mut tx = state.db.begin().await.map_err(db)?;
    service::put_report(&mut tx, &caller, id, &body.summary, &body.data).await?;
    tx.commit().await.map_err(db)?;
    Ok(StatusCode::NO_CONTENT)
}

/// What the plugins reported about a submitted attempt, by plugin. Staff and admins who may see
/// the attempt.
#[utoipa::path(
    get,
    path = "/submissions/{id}/plugin-reports",
    tag = "plugin-api",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "A submitted attempt")),
    responses(
        (status = 200, description = "The reports, by plugin", body = PluginReports),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the assignment but not its submissions",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such attempt, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn list_reports(
    actor: Actor,
    State(state): State<PluginsState>,
    Path(id): Path<Uuid>,
) -> Result<Json<PluginReports>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(db)?;
    let rows = service::reports(&mut conn, &actor, id).await?;
    Ok(Json(PluginReports {
        items: rows
            .into_iter()
            .map(|row| PluginReport {
                plugin_id: row.plugin_id,
                summary: row.summary,
                data: row.data,
                updated_at: row.updated_at,
            })
            .collect(),
    }))
}
