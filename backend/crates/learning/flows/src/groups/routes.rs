use access::ActorSource;
use api_types::{CourseLine, GroupMemberLine, GroupOverview, LinkedCourse, ProblemDetails};
use axum::Json;
use axum::extract::FromRef;
use axum::extract::{Path, State};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::service;
use crate::LearningError;
use crate::LearningState;
use access::Actor;
use common::ApiError;

pub fn router<S>() -> OpenApiRouter<S>
where
    S: ActorSource + Clone + 'static,
    LearningState: FromRef<S>,
{
    OpenApiRouter::new().routes(routes!(get_group_overview))
}

/// A group with its people (name, email, when they were added) and the courses that use it or one
/// of its subgroups. Admin only.
#[utoipa::path(
    get,
    path = "/admin/groups/{id}/overview",
    tag = "admin",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Group id")),
    responses(
        (status = 200, description = "The overview", body = GroupOverview),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such group", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn get_group_overview(
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<GroupOverview>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(LearningError::from)?;
    let overview = service::group_overview(&mut conn, &actor, id).await?;
    Ok(Json(GroupOverview {
        group_id: overview.group_id,
        student_count: i64::try_from(overview.members.len()).unwrap_or(i64::MAX),
        course_count: i64::try_from(overview.linked_courses.len()).unwrap_or(i64::MAX),
        members: overview
            .members
            .into_iter()
            .map(|member| GroupMemberLine {
                user_id: member.user_id,
                name: member.name,
                surname: member.surname,
                email: member.email,
                added_at: member.added_at,
            })
            .collect(),
        linked_courses: overview
            .linked_courses
            .into_iter()
            .map(|course| LinkedCourse {
                course: CourseLine {
                    course_id: course.course_id,
                    code: course.code,
                    title: course.title,
                },
                owner_name: course.owner_name,
                owner_surname: course.owner_surname,
                linked_on: course.linked_on,
            })
            .collect(),
    }))
}
