use api_types::{CourseLine, GroupName, ProblemDetails, UpdateUserRequest, User, UserFacts};
use axum::Json;
use axum::extract::{Path, State};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::service;
use crate::access::{self, Action};
use crate::actor::Actor;
use crate::app::AppState;
use crate::auth::{
    AuthError, PersonName, UserUpdate, api_role, api_status, api_user, domain_role, domain_status,
};
use crate::learning::LearningError;
use common::{ApiError, ValidJson};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(user_facts))
        .routes(routes!(update_user))
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

/// Changes a user's name, role or status. Admin only.
///
/// Disabling a user deletes all their sessions. The last active admin can be neither demoted
/// nor disabled.
#[utoipa::path(
    patch,
    path = "/admin/users/{id}",
    tag = "admin",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "User id")),
    request_body = UpdateUserRequest,
    responses(
        (status = 200, description = "The updated user", body = User),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such user", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 409, description = "Would remove the last active admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid name", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn update_user(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<UpdateUserRequest>,
) -> Result<Json<User>, ApiError> {
    access::authorize(&actor, Action::ManageUsers).map_err(LearningError::from)?;
    let update = UserUpdate {
        name: request
            .name
            .as_deref()
            .map(PersonName::name)
            .transpose()
            .map_err(AuthError::from)?,
        surname: request
            .surname
            .as_deref()
            .map(PersonName::surname)
            .transpose()
            .map_err(AuthError::from)?,
        role: request.role.map(domain_role),
        status: request.status.map(domain_status),
    };
    let mut tx = state.db.begin().await.map_err(LearningError::from)?;
    let user = service::update_user(&mut tx, &actor, id, update).await?;
    tx.commit().await.map_err(LearningError::from)?;
    Ok(Json(api_user(user)))
}
