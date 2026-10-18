//! The administrator's read models of groups and terms (ADR-0053).

use api_types::{
    CourseLine, GroupMemberLine, GroupOverview, LinkedCourse, ProblemDetails, TermStat, TermStats,
};
use axum::Json;
use axum::extract::{Path, State};
use chrono::Utc;
use uuid::Uuid;

use crate::actor::Actor;
use crate::app::AppState;
use crate::error::ApiError;
use crate::org::service::{self, OrgError};

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
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<GroupOverview>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(OrgError::from)?;
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
    let mut conn = state.db.acquire().await.map_err(OrgError::from)?;
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
