//! Publishing, deleting and copying a course (ADR-0048).

use api_types::{
    Course, CreateFromTemplateRequest, MakeTemplateRequest, Page, PageQuery, ProblemDetails,
    TemplateSummary,
};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use uuid::Uuid;

use crate::LearningError;
use crate::LearningState;
use crate::courses::service;
use crate::teaching::{CourseTitle, TeachingError, api_course};
use access::Actor;
use common::{ApiError, PageParams, ValidJson, ValidQuery};

/// Counts are far below `u32::MAX`; a database that disagreed would still not overflow the field.
fn count(n: i64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Publishes a draft course so its students can see it. Owner, course teacher or admin.
///
/// Idempotent: a published course is returned unchanged. Assistants cannot publish (ADR-0019).
#[utoipa::path(
    post,
    path = "/courses/{id}/publish",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    responses(
        (status = 200, description = "The published course", body = Course),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not publish this course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The course is archived", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn publish_course(
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<Course>, ApiError> {
    let mut tx = state.db.begin().await.map_err(LearningError::from)?;
    let course = service::publish(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(LearningError::from)?;
    Ok(Json(api_course(course)))
}

/// Deletes a course with its content, members and links. The owner or an admin, in every status.
///
/// A course in which any grade has a value or any work was submitted cannot be deleted: archive
/// it instead.
#[utoipa::path(
    delete,
    path = "/courses/{id}",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Only the owner or an admin may delete a course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The course has grades or submitted work",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn delete_course(
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(LearningError::from)?;
    service::delete(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(LearningError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The templates the caller owns or teaches, with how many sections, lectures and assignments
/// each holds.
#[utoipa::path(
    get,
    path = "/me/templates",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "A page of templates", body = Page<TemplateSummary>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn list_templates(
    State(state): State<LearningState>,
    actor: Actor,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<Page<TemplateSummary>>, ApiError> {
    let page = PageParams::new(query.cursor.as_deref(), query.limit)?;
    let mut conn = state.db.acquire().await.map_err(LearningError::from)?;
    let templates = service::templates(&mut conn, &actor, page).await?;
    Ok(Json(Page {
        items: templates
            .items
            .into_iter()
            .map(|template| TemplateSummary {
                course: api_course(template.course),
                sections: count(template.sections),
                lectures: count(template.lectures),
                assignments: count(template.assignments),
                updated_at: template.updated_at,
            })
            .collect(),
        next_cursor: templates.next.map(|id| id.to_string()),
    }))
}

/// Makes a template from a course: a copy of its whole tree, as drafts, owned by the caller.
///
/// Anyone who can edit the course's content and may create courses. Members, groups, deadlines,
/// submissions and grades are not copied. Works on an archived course.
#[utoipa::path(
    post,
    path = "/courses/{id}/template",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    request_body = MakeTemplateRequest,
    responses(
        (status = 201, description = "The new template", body = Course),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not do this", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid title", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn make_template(
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<MakeTemplateRequest>,
) -> Result<(StatusCode, Json<Course>), ApiError> {
    let title = request
        .title
        .as_deref()
        .map(CourseTitle::parse)
        .transpose()
        .map_err(TeachingError::from)?;
    let mut tx = state.db.begin().await.map_err(LearningError::from)?;
    let template = service::make_template(&mut tx, &actor, id, title).await?;
    tx.commit().await.map_err(LearningError::from)?;
    Ok((StatusCode::CREATED, Json(api_course(template))))
}

/// Makes a draft course from a template: a copy of its whole tree, as drafts, owned by the caller.
#[utoipa::path(
    post,
    path = "/templates/{id}/courses",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Template id")),
    request_body = CreateFromTemplateRequest,
    responses(
        (status = 201, description = "The new course", body = Course),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not do this", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such template, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid title or an unknown term", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn create_from_template(
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<CreateFromTemplateRequest>,
) -> Result<(StatusCode, Json<Course>), ApiError> {
    let title = request
        .title
        .as_deref()
        .map(CourseTitle::parse)
        .transpose()
        .map_err(TeachingError::from)?;
    let mut tx = state.db.begin().await.map_err(LearningError::from)?;
    let course = service::create_from_template(&mut tx, &actor, id, title, request.term_id).await?;
    tx.commit().await.map_err(LearningError::from)?;
    Ok((StatusCode::CREATED, Json(api_course(course))))
}
