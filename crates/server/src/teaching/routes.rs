//! HTTP edge of `teaching`: extractors, DTO mapping, OpenAPI annotations. Writes open the
//! transaction, call exactly one service function and commit; reads use a pooled connection.

use api_types::{
    Course, CreateCourseRequest, Membership, Page, PageQuery, ProblemDetails, PutMemberRequest,
};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::model::{self, CourseKind, CourseTitle, ModelError};
use super::service::{self, TeachingError};
use crate::actor::Actor;
use crate::app::AppState;
use crate::error::{ApiError, ValidJson, ValidQuery};
use crate::identity::PageParams;
use crate::policy::{CourseRole, CourseStatus};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(create_course))
        .routes(routes!(get_course))
        .routes(routes!(publish_course))
        .routes(routes!(list_my_courses))
        .routes(routes!(list_members))
        .routes(routes!(put_member, remove_member))
}

impl From<TeachingError> for ApiError {
    fn from(error: TeachingError) -> Self {
        match error {
            TeachingError::Forbidden(_) => Self::Forbidden("not allowed"),
            TeachingError::Invalid(ModelError::Title) => {
                Self::invalid_field("title", error.to_string())
            }
            TeachingError::CourseNotFound | TeachingError::MemberNotFound => Self::NotFound,
            TeachingError::UserNotFound => Self::invalid_field("user_id", "no such user"),
            TeachingError::UserDisabled => Self::invalid_field("user_id", "that user is disabled"),
            TeachingError::RoleNotAllowed => {
                Self::invalid_field("role", "this user cannot hold that course role")
            }
            TeachingError::OwnerMustStayTeacher => {
                Self::Conflict("the course owner must stay a teacher")
            }
            TeachingError::OwnerCannotBeRemoved => {
                Self::Conflict("the course owner cannot be removed")
            }
            TeachingError::NotPublishable => {
                Self::Conflict("an archived course cannot be published")
            }
            TeachingError::Identity(e) => e.into(),
            TeachingError::Events(e) => {
                tracing::error!(error = %e, "teaching: audit log error");
                Self::Internal
            }
            TeachingError::Db(e) => {
                tracing::error!(error = %e, "teaching: database error");
                Self::Internal
            }
        }
    }
}

fn api_kind(kind: CourseKind) -> api_types::CourseKind {
    match kind {
        CourseKind::Regular => api_types::CourseKind::Regular,
        CourseKind::Template => api_types::CourseKind::Template,
    }
}

fn api_status(status: CourseStatus) -> api_types::CourseStatus {
    match status {
        CourseStatus::Draft => api_types::CourseStatus::Draft,
        CourseStatus::Published => api_types::CourseStatus::Published,
        CourseStatus::Archived => api_types::CourseStatus::Archived,
    }
}

fn api_role(role: CourseRole) -> api_types::CourseRole {
    match role {
        CourseRole::Teacher => api_types::CourseRole::Teacher,
        CourseRole::Assistant => api_types::CourseRole::Assistant,
        CourseRole::Student => api_types::CourseRole::Student,
    }
}

fn domain_role(role: api_types::CourseRole) -> CourseRole {
    match role {
        api_types::CourseRole::Teacher => CourseRole::Teacher,
        api_types::CourseRole::Assistant => CourseRole::Assistant,
        api_types::CourseRole::Student => CourseRole::Student,
    }
}

fn api_course(course: model::Course) -> Course {
    Course {
        id: course.id,
        kind: api_kind(course.kind),
        title: course.title,
        status: api_status(course.status),
        owner_id: course.owner_id,
        created_at: course.created_at,
    }
}

fn api_membership(member: model::Membership) -> Membership {
    Membership {
        course_id: member.course_id,
        user_id: member.user_id,
        role: api_role(member.role),
        is_owner: member.is_owner,
    }
}

fn page_params(query: &PageQuery) -> Result<PageParams, ApiError> {
    Ok(PageParams::new(query.cursor.as_deref(), query.limit)?)
}

/// Creates a draft course owned by the caller. Teachers and admins only.
///
/// A teacher joins the course as its `teacher`. An admin owns it without a membership.
#[utoipa::path(
    post,
    path = "/courses",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CreateCourseRequest,
    responses(
        (status = 201, description = "The new draft course", body = Course),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not a teacher or admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid title", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn create_course(
    State(state): State<AppState>,
    actor: Actor,
    ValidJson(request): ValidJson<CreateCourseRequest>,
) -> Result<(StatusCode, Json<Course>), ApiError> {
    let title = CourseTitle::parse(&request.title).map_err(TeachingError::from)?;
    let mut tx = state.db.begin().await.map_err(TeachingError::from)?;
    let course = service::create_course(&mut tx, &actor, title).await?;
    tx.commit().await.map_err(TeachingError::from)?;
    Ok((StatusCode::CREATED, Json(api_course(course))))
}

/// One course. Students see it only once it is published; anyone who may not see it gets 404.
#[utoipa::path(
    get,
    path = "/courses/{id}",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    responses(
        (status = 200, description = "The course", body = Course),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn get_course(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<Course>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(TeachingError::from)?;
    let course = service::get_course(&mut conn, &actor, id).await?;
    Ok(Json(api_course(course)))
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
async fn publish_course(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<Course>, ApiError> {
    let mut tx = state.db.begin().await.map_err(TeachingError::from)?;
    let course = service::publish_course(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(TeachingError::from)?;
    Ok(Json(api_course(course)))
}

/// The courses the caller owns or belongs to, in creation order.
///
/// A student does not see draft courses.
#[utoipa::path(
    get,
    path = "/me/courses",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "A page of courses", body = Page<Course>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn list_my_courses(
    State(state): State<AppState>,
    actor: Actor,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<Page<Course>>, ApiError> {
    let page = page_params(&query)?;
    let mut conn = state.db.acquire().await.map_err(TeachingError::from)?;
    let courses = service::list_my_courses(&mut conn, &actor, page).await?;
    Ok(Json(Page {
        items: courses.items.into_iter().map(api_course).collect(),
        next_cursor: courses.next.map(|id| id.to_string()),
    }))
}

/// The members of a course, ordered by user id. Anyone who can see the course can see this.
///
/// Entries carry ids only. The course owner is a `teacher` with `is_owner` set, except when an
/// admin owns the course: then the owner has no membership and is not listed.
#[utoipa::path(
    get,
    path = "/courses/{id}/members",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id"), PageQuery),
    responses(
        (status = 200, description = "A page of members", body = Page<Membership>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn list_members(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<Page<Membership>>, ApiError> {
    let page = page_params(&query)?;
    let mut conn = state.db.acquire().await.map_err(TeachingError::from)?;
    let members = service::list_members(&mut conn, &actor, id, page).await?;
    Ok(Json(Page {
        items: members.items.into_iter().map(api_membership).collect(),
        next_cursor: members.next.map(|id| id.to_string()),
    }))
}

/// Adds a user to a course or changes their role.
///
/// Idempotent: putting the role the user already has changes nothing. Staff manage students;
/// only the owner, a course teacher or an admin manage teachers and assistants. A `teacher`
/// must be a teacher at the institution, and the owner must stay a teacher. Works on draft
/// courses, so students can be enrolled before publishing.
#[utoipa::path(
    put,
    path = "/courses/{id}/members/{user_id}",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Course id"),
        ("user_id" = Uuid, Path, description = "The user to add or change"),
    ),
    request_body = PutMemberRequest,
    responses(
        (status = 200, description = "The membership", body = Membership),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not manage members with this role",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The owner must stay a teacher", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Unknown user, or a role the user cannot hold",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn put_member(
    State(state): State<AppState>,
    actor: Actor,
    Path((course_id, user_id)): Path<(Uuid, Uuid)>,
    ValidJson(request): ValidJson<PutMemberRequest>,
) -> Result<Json<Membership>, ApiError> {
    let mut tx = state.db.begin().await.map_err(TeachingError::from)?;
    let member = service::put_member(
        &mut tx,
        &actor,
        course_id,
        user_id,
        domain_role(request.role),
    )
    .await?;
    tx.commit().await.map_err(TeachingError::from)?;
    Ok(Json(api_membership(member)))
}

/// Removes a member from a course. The owner cannot be removed.
#[utoipa::path(
    delete,
    path = "/courses/{id}/members/{user_id}",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Course id"),
        ("user_id" = Uuid, Path, description = "The member to remove"),
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not manage members with this role",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course or member", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 409, description = "The owner cannot be removed", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn remove_member(
    State(state): State<AppState>,
    actor: Actor,
    Path((course_id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(TeachingError::from)?;
    service::remove_member(&mut tx, &actor, course_id, user_id).await?;
    tx.commit().await.map_err(TeachingError::from)?;
    Ok(StatusCode::NO_CONTENT)
}
