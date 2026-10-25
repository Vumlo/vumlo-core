//! HTTP edge of the plugin API (ADR-0066): extractors, DTO mapping, OpenAPI annotations. A plugin
//! calls `/plugin-api/*` with its token; every handler checks a scope in the service it calls.
//! Writes open the transaction, call exactly one writing service function and commit; the reads
//! that decide whether the write may go ahead run before it.

use access::{Actor, ActorSource, PluginCaller, PluginSource};
use api_types::{
    Launch, LaunchRequest, PluginReport, PluginReports, PluginSlot, PluginSlots, PluginSubmission,
    ProblemDetails, PutReportRequest, SlotQuery,
};
use axum::Json;
use axum::extract::{FromRef, Path, State};
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use chrono::{TimeDelta, Utc};
use common::{ApiError, ValidJson, ValidQuery};
use learning::assessment;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::model::SLOTS;
use crate::secret::LAUNCH_TTL_SECONDS;
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
        .routes(routes!(list_slots))
        .routes(routes!(launch))
}

/// Responses that carry a credential must not be cached (AGENTS section 11).
fn no_store() -> (HeaderName, HeaderValue) {
    (header::CACHE_CONTROL, HeaderValue::from_static("no-store"))
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

/// The pages that enabled plugins offer in a slot of the web app. Any signed-in user. The web app
/// frames nothing but these addresses.
#[utoipa::path(
    get,
    path = "/plugin-slots",
    tag = "plugin-api",
    security(("cookie" = []), ("bearer" = [])),
    params(SlotQuery),
    responses(
        (status = 200, description = "The pages, by plugin", body = PluginSlots),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "No such slot", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn list_slots(
    _actor: Actor,
    State(state): State<PluginsState>,
    ValidQuery(query): ValidQuery<SlotQuery>,
) -> Result<Json<PluginSlots>, ApiError> {
    if !SLOTS.contains(&query.slot.as_str()) {
        return Err(ApiError::invalid_field(
            "slot",
            format!("must be one of: {}", SLOTS.join(", ")),
        ));
    }
    let mut conn = state.db.acquire().await.map_err(db)?;
    let pages = service::slots(&mut conn, &query.slot).await?;
    Ok(Json(PluginSlots {
        items: pages
            .into_iter()
            .map(|page| PluginSlot {
                plugin_id: page.plugin_id,
                title: page.title,
                url: page.url,
            })
            .collect(),
    }))
}

/// A token that lets the plugin's page learn who looks at this assignment: its own server checks
/// the signature with the secret it has. Valid for a minute; ask again for a fresh one. Anyone who
/// can see the assignment.
#[utoipa::path(
    post,
    path = "/assignments/{id}/plugin-launch",
    tag = "plugin-api",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Assignment id")),
    request_body = LaunchRequest,
    responses(
        (status = 200, description = "The token", body = Launch),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such assignment, or not visible to the caller, or no such \
            plugin, or it offers no tab", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "No plugin id", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn launch(
    actor: Actor,
    State(state): State<PluginsState>,
    Path(id): Path<Uuid>,
    ValidJson(body): ValidJson<LaunchRequest>,
) -> Result<([(HeaderName, HeaderValue); 1], Json<Launch>), ApiError> {
    // Without the key no plugin is installed, so there is nothing to launch.
    let key = state.key.as_deref().ok_or(ApiError::NotFound)?;
    let now = Utc::now();
    let mut conn = state.db.acquire().await.map_err(db)?;
    let token =
        service::launch(&mut conn, &actor, key, id, &body.plugin_id, now.timestamp()).await?;
    Ok((
        [no_store()],
        Json(Launch {
            token,
            expires_at: now + TimeDelta::seconds(LAUNCH_TTL_SECONDS),
        }),
    ))
}
