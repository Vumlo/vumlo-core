use api_types::{CourseLine, GroupName, ProblemDetails, UserFacts};
use axum::Json;
use axum::extract::{Path, State};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::service;
use crate::actor::Actor;
use crate::app::AppState;
use crate::auth::{api_role, api_status};
use crate::learning::LearningError;
use common::ApiError;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(user_facts))
}

/// What one person does in the institution: the groups they are in, the courses they own, teach
/// and study in, and when they last signed in. No email appears here. Admin only.
#[utoipa::path(
    get,
    path = "/admin/users/{id}/facts",
    tag = "admin",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "User id")),
    responses(
        (status = 200, description = "The facts", body = UserFacts),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such user", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn user_facts(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<UserFacts>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(LearningError::from)?;
    let facts = service::user_facts(&mut conn, &actor, id).await?;
    let line = |course: crate::learning::teaching::CourseLineRow| CourseLine {
        course_id: course.course_id,
        code: course.code,
        title: course.title,
    };
    Ok(Json(UserFacts {
        role: api_role(facts.user.role),
        status: api_status(facts.user.status),
        created_at: facts.user.created_at,
        last_login_at: facts.last_login_at,
        groups: facts
            .groups
            .into_iter()
            .map(|(id, name)| GroupName { id, name })
            .collect(),
        owns: facts.courses.owns.into_iter().map(line).collect(),
        teaches: facts.courses.teaches.into_iter().map(line).collect(),
        studies: facts.courses.studies,
    }))
}
