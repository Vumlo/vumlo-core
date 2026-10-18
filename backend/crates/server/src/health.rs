use api_types::{HealthResponse, ProblemDetails};
use axum::Json;
use axum::extract::State;

use crate::app::AppState;
use crate::error::ApiError;

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
