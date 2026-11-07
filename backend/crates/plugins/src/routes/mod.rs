//! HTTP edge of the plugin API (ADR-0066): extractors, DTO mapping, OpenAPI annotations. A plugin
//! calls `/plugin-api/*` with its token; every handler checks a scope in the service it calls.
//! Writes open the transaction, call exactly one writing service function and commit; the reads
//! that decide whether the write may go ahead run before it.

use access::{Actor, ActorSource, PluginCaller, PluginSource};
use api_types::{
    Launch, LaunchRequest, PluginSettings, PluginSlot, PluginSlots, ProblemDetails, SettingsQuery,
    SlotQuery,
};
use axum::Json;
use axum::extract::{FromRef, Path, State};
use axum::http::{HeaderName, HeaderValue, header};
use chrono::{TimeDelta, Utc};
use common::{ApiError, ValidJson, ValidQuery};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use vumlo_config::Settings;

use crate::PluginsError;
use crate::model::SLOTS;
use crate::secret::LAUNCH_TTL_SECONDS;
use crate::service;
use crate::state::PluginsState;

mod guard;
mod reads;
mod reports;

/// Everything under `/plugin-api` has an allowance of calls per token and a line in the log
/// (ADR-0079); the calls staff and the web app make about plugins do not.
pub fn router<S>(
    state: &PluginsState,
    settings: &Settings,
) -> Result<OpenApiRouter<S>, PluginsError>
where
    S: PluginSource + ActorSource + Clone + 'static,
    PluginsState: FromRef<S>,
{
    // The limiter is the inner layer and the guard the outer: the guard finds the plugin the token
    // belongs to, which is what the limiter counts by.
    let plugin_api = OpenApiRouter::new()
        .merge(reads::router())
        .merge(reports::plugin_router())
        .routes(routes!(get_settings));
    let plugin_api = guard::limit(plugin_api, settings)?.layer(
        axum::middleware::from_fn_with_state(state.clone(), guard::record),
    );
    Ok(OpenApiRouter::new()
        .merge(plugin_api)
        .merge(reports::staff_router())
        .routes(routes!(list_slots))
        .routes(routes!(launch)))
}

/// Responses that carry a credential must not be cached (AGENTS section 11).
pub(crate) fn no_store() -> (HeaderName, HeaderValue) {
    (header::CACHE_CONTROL, HeaderValue::from_static("no-store"))
}

pub(crate) fn db(error: sqlx::Error) -> ApiError {
    ApiError::from_db("plugins", &error)
}

/// The plugin's own settings: what an administrator configured, secrets included, with the manifest's
/// defaults where nothing was set. The scope `settings:read`. The settings of a course come with
/// `course_id` (`404` for a course that is not there). A plugin learns of a change from its `plugin.settings.changed` event.
#[utoipa::path(
    get,
    path = "/plugin-api/settings",
    tag = "plugin-api",
    security(("plugin" = [])),
    params(SettingsQuery),
    responses(
        (status = 200, description = "The values", body = PluginSettings),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `settings:read`",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "`course_id` names no course", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn get_settings(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    ValidQuery(query): ValidQuery<SettingsQuery>,
) -> Result<([(HeaderName, HeaderValue); 1], Json<PluginSettings>), ApiError> {
    // Without the key no secret can be read, and no plugin was installed.
    let key = state.key.as_deref().ok_or(PluginsError::NoKey)?;
    let mut conn = state.db.acquire().await.map_err(db)?;
    let values = service::settings_for_plugin(&mut conn, &caller, key, query.course_id).await?;
    // The values may hold a credential (AGENTS section 11).
    Ok(([no_store()], Json(PluginSettings { values })))
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
