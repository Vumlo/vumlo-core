use api_types::{HealthResponse, MetaResponse, ModuleMeta, ProblemDetails};
use axum::Json;
use axum::extract::State;

use crate::app::AppState;
use common::ApiError;

#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "The service and its database are healthy", body = HealthResponse),
        (status = 503, description = "The database does not answer", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn health(State(state): State<AppState>) -> Result<Json<HealthResponse>, ApiError> {
    sqlx::query!(r#"SELECT 1 AS "ok!""#)
        .fetch_one(&state.db)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "health check: database unreachable");
            ApiError::ServiceUnavailable("database unavailable")
        })?;
    Ok(Json(HealthResponse {
        status: "ok".to_owned(),
        database: "ok".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        modules: crate::modules::compiled_names()
            .into_iter()
            .map(str::to_owned)
            .collect(),
    }))
}

/// Which optional modules are on. Public: it names the modules and their versions, nothing else.
#[utoipa::path(
    get,
    path = "/meta",
    responses((status = 200, description = "The enabled optional modules, in start order", body = MetaResponse))
)]
pub async fn meta(State(state): State<AppState>) -> Json<MetaResponse> {
    Json(MetaResponse {
        modules: state
            .modules
            .iter()
            .map(|module| module.manifest())
            .filter(|manifest| state.config.module_state.is_enabled(manifest.id))
            .map(|manifest| ModuleMeta {
                id: manifest.id.to_owned(),
                version: manifest.version.to_owned(),
            })
            .collect(),
    })
}
