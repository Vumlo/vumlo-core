//! The routes of the console that administer plugins (ADR-0095). Every handler takes an `Actor` and
//! the service it calls authorizes: only a console administrator may. A write opens the transaction,
//! calls one writing service function and commits.

use access::{Actor, CourseScope, Scope};
use api_types::{
    AdminPlugin, AdminPluginCall, AdminPluginCallCount, AdminPluginCalls, AdminPluginCallsQuery,
    AdminPluginSetting, AdminPluginSettings, AdminPlugins, GrantPluginRequest, PluginStatus,
    ProblemDetails, SetPluginSettingRequest, SetPluginStatusRequest,
};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{Duration, Utc};
use common::{ApiError, ValidJson, ValidQuery};
use sqlx::PgPool;

use super::db;
use crate::admin::{self, AdminInstall};
use crate::error::PluginsError;
use crate::model::{Install, InstallStatus};
use crate::service::SettingView;
use crate::settings::{ScalarKind, SettingKind};
use crate::state::PluginsState;

fn status(status: InstallStatus) -> PluginStatus {
    match status {
        InstallStatus::Enabled => PluginStatus::Enabled,
        InstallStatus::Disabled => PluginStatus::Disabled,
    }
}

fn names(scopes: &[Scope]) -> Vec<String> {
    scopes
        .iter()
        .map(|scope| scope.as_str().to_owned())
        .collect()
}

fn api_install(
    install: &Install,
    pending: i64,
    failed: i64,
    needs_setup: Vec<String>,
) -> AdminPlugin {
    let (all_courses, course_ids) = match &install.courses {
        CourseScope::All => (true, Vec::new()),
        CourseScope::Only(list) => (false, list.clone()),
    };
    AdminPlugin {
        plugin_id: install.plugin_id.clone(),
        version: install.version.clone(),
        base_url: install.base_url.clone(),
        status: status(install.status),
        requested_scopes: names(&install.requested_scopes),
        scopes: names(&install.scopes),
        all_courses,
        course_ids,
        pending,
        failed,
        needs_setup,
    }
}

fn api_admin(found: AdminInstall) -> AdminPlugin {
    let AdminInstall {
        summary,
        needs_setup,
    } = found;
    api_install(
        &summary.install,
        summary.pending,
        summary.failed,
        needs_setup,
    )
}

fn api_setting(view: SettingView) -> AdminPluginSetting {
    let kind = match view.spec.kind {
        SettingKind::String => "string",
        SettingKind::Number => "number",
        SettingKind::Boolean => "boolean",
        SettingKind::Array => "array",
    };
    let item_kind = view.spec.items.as_ref().map(|item| match item.kind {
        ScalarKind::String => "string".to_owned(),
        ScalarKind::Number => "number".to_owned(),
    });
    AdminPluginSetting {
        key: view.key,
        description: view.spec.description.clone(),
        kind: kind.to_owned(),
        secret: view.spec.secret,
        options: view.spec.options.clone(),
        minimum: view.spec.minimum,
        maximum: view.spec.maximum,
        item_kind,
        default: if view.spec.secret {
            None
        } else {
            view.spec.default.clone()
        },
        is_set: view.is_set,
        // The view holds no value for a secret; this keeps it so whatever the view does.
        value: if view.spec.secret { None } else { view.value },
    }
}

async fn connection(pool: &PgPool) -> Result<sqlx::pool::PoolConnection<sqlx::Postgres>, ApiError> {
    pool.acquire().await.map_err(db)
}

fn domain_status(status: PluginStatus) -> InstallStatus {
    match status {
        PluginStatus::Enabled => InstallStatus::Enabled,
        PluginStatus::Disabled => InstallStatus::Disabled,
    }
}

/// The installed plugins, with their state, what they were granted, their deliveries and which
/// settings still need a value. Console administrators only.
#[utoipa::path(
    get,
    path = "/admin/plugins",
    tag = "admin",
    security(("console" = [])),
    responses(
        (status = 200, description = "The installs", body = AdminPlugins),
        (status = 401, description = "No console session", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not a console administrator", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub(super) async fn list_plugins(
    actor: Actor,
    State(state): State<PluginsState>,
) -> Result<Json<AdminPlugins>, ApiError> {
    let mut conn = connection(&state.db).await?;
    let items = admin::admin_list(&mut conn, &actor)
        .await?
        .into_iter()
        .map(api_admin)
        .collect();
    Ok(Json(AdminPlugins { items }))
}

/// One install. Console administrators only.
#[utoipa::path(
    get,
    path = "/admin/plugins/{id}",
    tag = "admin",
    security(("console" = [])),
    params(("id" = String, Path, description = "Plugin id")),
    responses(
        (status = 200, description = "The install", body = AdminPlugin),
        (status = 403, description = "Not a console administrator", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such plugin", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub(super) async fn get_plugin(
    actor: Actor,
    State(state): State<PluginsState>,
    Path(plugin_id): Path<String>,
) -> Result<Json<AdminPlugin>, ApiError> {
    let mut conn = connection(&state.db).await?;
    Ok(Json(api_admin(
        admin::admin_get(&mut conn, &actor, &plugin_id).await?,
    )))
}

/// Switches a plugin on or off. Off, its token stops working, it is sent no event and its queued
/// mail is cancelled (ADR-0085). Console administrators only.
#[utoipa::path(
    put,
    path = "/admin/plugins/{id}/status",
    tag = "admin",
    security(("console" = [])),
    params(("id" = String, Path, description = "Plugin id")),
    request_body = SetPluginStatusRequest,
    responses(
        (status = 200, description = "The install after the change", body = AdminPlugin),
        (status = 403, description = "Not a console administrator", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such plugin", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub(super) async fn set_status(
    actor: Actor,
    State(state): State<PluginsState>,
    Path(plugin_id): Path<String>,
    ValidJson(request): ValidJson<SetPluginStatusRequest>,
) -> Result<Json<AdminPlugin>, ApiError> {
    let mut tx = state.db.begin().await.map_err(db)?;
    let install =
        admin::admin_set_status(&mut tx, &actor, &plugin_id, domain_status(request.status)).await?;
    tx.commit().await.map_err(db)?;
    let mut conn = connection(&state.db).await?;
    Ok(Json(api_admin(
        admin::admin_get(&mut conn, &actor, &install.plugin_id).await?,
    )))
}

/// Replaces what a plugin may do and reach. The scopes stay within what it asked for. Console
/// administrators only.
#[utoipa::path(
    put,
    path = "/admin/plugins/{id}/grants",
    tag = "admin",
    security(("console" = [])),
    params(("id" = String, Path, description = "Plugin id")),
    request_body = GrantPluginRequest,
    responses(
        (status = 200, description = "The install after the change", body = AdminPlugin),
        (status = 403, description = "Not a console administrator", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such plugin, or a course that is not there",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A scope that is unknown or that the plugin did not ask for",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub(super) async fn grant(
    actor: Actor,
    State(state): State<PluginsState>,
    Path(plugin_id): Path<String>,
    ValidJson(request): ValidJson<GrantPluginRequest>,
) -> Result<Json<AdminPlugin>, ApiError> {
    let scopes = request
        .scopes
        .iter()
        .map(|name| {
            name.parse::<Scope>()
                .map_err(|_| ApiError::invalid_field("scopes", format!("`{name}` is not a scope")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let courses = if request.all_courses {
        CourseScope::All
    } else {
        CourseScope::Only(request.course_ids)
    };
    let mut tx = state.db.begin().await.map_err(db)?;
    admin::admin_grant(&mut tx, &actor, &plugin_id, &scopes, &courses).await?;
    tx.commit().await.map_err(db)?;
    let mut conn = connection(&state.db).await?;
    Ok(Json(api_admin(
        admin::admin_get(&mut conn, &actor, &plugin_id).await?,
    )))
}

/// The institution settings of a plugin, as its manifest declares them. A secret shows only
/// whether it is set. Console administrators only.
#[utoipa::path(
    get,
    path = "/admin/plugins/{id}/settings",
    tag = "admin",
    security(("console" = [])),
    params(("id" = String, Path, description = "Plugin id")),
    responses(
        (status = 200, description = "The settings", body = AdminPluginSettings),
        (status = 403, description = "Not a console administrator", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such plugin", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub(super) async fn list_settings(
    actor: Actor,
    State(state): State<PluginsState>,
    Path(plugin_id): Path<String>,
) -> Result<Json<AdminPluginSettings>, ApiError> {
    let mut conn = connection(&state.db).await?;
    let mut items: Vec<AdminPluginSetting> = admin::admin_settings(&mut conn, &actor, &plugin_id)
        .await?
        .into_iter()
        .map(api_setting)
        .collect();
    items.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(Json(AdminPluginSettings { items }))
}

/// Sets one institution setting from text: a string stays text, a number, a boolean or a list is
/// JSON. The value is checked against the manifest's spec and never returned. Console
/// administrators only.
#[utoipa::path(
    put,
    path = "/admin/plugins/{id}/settings/{key}",
    tag = "admin",
    security(("console" = [])),
    params(
        ("id" = String, Path, description = "Plugin id"),
        ("key" = String, Path, description = "Setting key"),
    ),
    request_body = SetPluginSettingRequest,
    responses(
        (status = 204, description = "Stored, or it had this value already"),
        (status = 403, description = "Not a console administrator", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such plugin or setting", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "The value does not meet the setting's spec, or the setting \
            belongs to a course", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub(super) async fn set_setting(
    actor: Actor,
    State(state): State<PluginsState>,
    Path((plugin_id, key)): Path<(String, String)>,
    ValidJson(request): ValidJson<SetPluginSettingRequest>,
) -> Result<StatusCode, ApiError> {
    // Without the key no secret can be sealed, and no plugin was installed.
    let secret_key = state.key.as_deref().ok_or(PluginsError::NoKey)?;
    let mut tx = state.db.begin().await.map_err(db)?;
    admin::admin_set_setting(
        &mut tx,
        &actor,
        secret_key,
        &plugin_id,
        &key,
        &request.value,
    )
    .await?;
    tx.commit().await.map_err(db)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Takes a value away, so that the default applies again. Console administrators only.
#[utoipa::path(
    delete,
    path = "/admin/plugins/{id}/settings/{key}",
    tag = "admin",
    security(("console" = [])),
    params(
        ("id" = String, Path, description = "Plugin id"),
        ("key" = String, Path, description = "Setting key"),
    ),
    responses(
        (status = 204, description = "Removed, or there was no value"),
        (status = 403, description = "Not a console administrator", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such plugin or setting", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "The setting belongs to a course", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub(super) async fn unset_setting(
    actor: Actor,
    State(state): State<PluginsState>,
    Path((plugin_id, key)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(db)?;
    admin::admin_unset_setting(&mut tx, &actor, &plugin_id, &key).await?;
    tx.commit().await.map_err(db)?;
    Ok(StatusCode::NO_CONTENT)
}

/// What the plugin did to the plugin API (ADR-0079): its newest calls and the counts by route and
/// answer. Console administrators only.
#[utoipa::path(
    get,
    path = "/admin/plugins/{id}/calls",
    tag = "admin",
    security(("console" = [])),
    params(("id" = String, Path, description = "Plugin id"), AdminPluginCallsQuery),
    responses(
        (status = 200, description = "The report", body = AdminPluginCalls),
        (status = 403, description = "Not a console administrator", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such plugin", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub(super) async fn calls(
    actor: Actor,
    State(state): State<PluginsState>,
    Path(plugin_id): Path<String>,
    ValidQuery(query): ValidQuery<AdminPluginCallsQuery>,
) -> Result<Json<AdminPluginCalls>, ApiError> {
    let since = query
        .since
        .unwrap_or_else(|| Utc::now() - Duration::hours(24));
    let mut conn = connection(&state.db).await?;
    let report = admin::admin_calls(
        &mut conn,
        &actor,
        &plugin_id,
        since,
        query.limit.unwrap_or(50),
    )
    .await?;
    Ok(Json(AdminPluginCalls {
        recent: report
            .recent
            .into_iter()
            .map(|call| AdminPluginCall {
                at: call.at,
                method: call.method,
                route: call.route,
                status: call.status,
                duration_ms: call.duration_ms,
            })
            .collect(),
        counts: report
            .counts
            .into_iter()
            .map(|count| AdminPluginCallCount {
                method: count.method,
                route: count.route,
                status: count.status,
                calls: count.calls,
                average_ms: count.average_ms,
            })
            .collect(),
    }))
}
