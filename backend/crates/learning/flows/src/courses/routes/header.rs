//! The header of a course and the people on it (ADR-0048).

use api_types::{CourseInfo, CoursePeople, PersonRow, ProblemDetails, UpdateCourseInfoRequest};
use axum::Json;
use axum::extract::{Path, State};
use sqlx::types::Decimal;
use uuid::Uuid;

use crate::LearningError;
use crate::LearningState;
use crate::courses::model;
use crate::courses::service;
use crate::teaching::{CourseInfoUpdate, api_linked_group, api_person};
use access::Actor;
use common::{ApiError, ValidJson};

/// Counts are far below `u32::MAX`; a database that disagreed would still not overflow the field.
fn count(n: i64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Points always have two decimals (ADR-0021).
fn points(value: Decimal) -> String {
    format!("{value:.2}")
}

fn api_info(info: model::CourseInfo) -> CourseInfo {
    CourseInfo {
        course_id: info.course_id,
        code: info.code,
        description: info.description,
        term_name: info.term_name,
        owner_name: info.owner_name,
        owner_surname: info.owner_surname,
        owner_email: info.owner_email,
        student_count: count(info.student_count),
        former_count: count(info.former_count),
        assistant_count: count(info.assistant_count),
        updated_at: info.updated_at,
        content_updated_at: info.content_updated_at,
        total_points: points(info.total_points),
        graded_items: count(info.graded_items),
        has_grades: info.has_grades,
    }
}

/// The header of a course: code, description, term, owner, counts and the points it adds up to.
/// Anyone who may view the course.
#[utoipa::path(
    get,
    path = "/courses/{id}/info",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    responses(
        (status = 200, description = "The course header", body = CourseInfo),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn get_course_info(
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<CourseInfo>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(LearningError::from)?;
    let info = service::info(&mut conn, &actor, id).await?;
    Ok(Json(api_info(info)))
}

/// Changes the code and/or the description of a course. Assistants, teachers, owners and admins;
/// not in an archived course. Only the fields present change; an empty one clears the field.
#[utoipa::path(
    patch,
    path = "/courses/{id}/info",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    request_body = UpdateCourseInfoRequest,
    responses(
        (status = 200, description = "The course header after the change", body = CourseInfo),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not edit this course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The course is archived", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "A code or a description that is too long",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn update_course_info(
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<UpdateCourseInfoRequest>,
) -> Result<Json<CourseInfo>, ApiError> {
    let update = CourseInfoUpdate {
        code: request.code,
        description: request.description,
    };
    let mut tx = state.db.begin().await.map_err(LearningError::from)?;
    let info = service::update_info(&mut tx, &actor, id, update).await?;
    tx.commit().await.map_err(LearningError::from)?;
    Ok(Json(api_info(info)))
}

/// The people on a course with their names. Anyone who may view the course.
///
/// Staff also get the emails, the former students (ADR-0037) and the linked groups; a student gets
/// names and groups of classmates and no address.
#[utoipa::path(
    get,
    path = "/courses/{id}/people",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    responses(
        (status = 200, description = "The people", body = CoursePeople),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn get_people(
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<CoursePeople>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(LearningError::from)?;
    let people = service::people(&mut conn, &actor, id).await?;
    Ok(Json(CoursePeople {
        people: people
            .people
            .into_iter()
            .map(api_person)
            .collect::<Vec<PersonRow>>(),
        groups: people.groups.into_iter().map(api_linked_group).collect(),
    }))
}
