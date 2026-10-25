use api_types::{ProblemDetails, TermStat, TermStats};
use axum::Json;
use axum::extract::State;
use chrono::Utc;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::service;
use crate::actor::Actor;
use crate::app::AppState;
use crate::learning::LearningError;
use common::ApiError;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_term_stats))
}

/// How many regular courses each term has, whether today (in the institution's time zone) falls in
/// it and whether it is over. Admin only.
#[utoipa::path(
    get,
    path = "/admin/term-stats",
    tag = "admin",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "One entry per term, newest first", body = TermStats),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn get_term_stats(
    State(state): State<AppState>,
    actor: Actor,
) -> Result<Json<TermStats>, ApiError> {
    let today = Utc::now()
        .with_timezone(&state.config.institution.time_zone)
        .date_naive();
    let mut conn = state.db.acquire().await.map_err(LearningError::from)?;
    let usage = service::term_stats(&mut conn, &actor, today).await?;
    Ok(Json(TermStats {
        items: usage
            .into_iter()
            .map(|term| TermStat {
                term_id: term.term_id,
                courses: term.courses,
                current: term.current,
                read_only: term.ended,
            })
            .collect(),
    }))
}
