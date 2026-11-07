//! Course announcements (ADR-0076).

use api_types::{
    Announcement, CreateAnnouncementRequest, Page, PageQuery, ProblemDetails,
    UpdateAnnouncementRequest,
};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use uuid::Uuid;

use super::page_params;
use crate::LearningState;
use crate::teaching::model;
use crate::teaching::service::{self, TeachingError};
use access::Actor;
use common::{ApiError, ValidJson, ValidQuery};

fn api_announcement(a: model::Announcement) -> Announcement {
    Announcement {
        id: a.id,
        course_id: a.course_id,
        author_id: a.author_id,
        title: a.title,
        body: a.body,
        created_at: a.created_at,
        updated_at: a.updated_at,
    }
}

/// The announcements of a course, newest first. Anyone who can see the course.
#[utoipa::path(
    get,
    path = "/courses/{id}/announcements",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id"), PageQuery),
    responses(
        (status = 200, description = "A page of announcements", body = Page<Announcement>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn list_announcements(
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<Page<Announcement>>, ApiError> {
    let page = page_params(&query)?;
    let mut conn = state.db.acquire().await.map_err(TeachingError::from)?;
    let rows = service::list_announcements(&mut conn, &actor, id, page).await?;
    Ok(Json(Page {
        items: rows.items.into_iter().map(api_announcement).collect(),
        next_cursor: rows.next.map(|id| id.to_string()),
    }))
}

/// Publishes an announcement to the course at once, and tells the plugins that subscribed to
/// `announcement.published`. Staff and admins; not in an archived course.
#[utoipa::path(
    post,
    path = "/courses/{id}/announcements",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    request_body = CreateAnnouncementRequest,
    responses(
        (status = 201, description = "Published", body = Announcement),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not write content in this course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The course is archived", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid title or text", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn publish_announcement(
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(body): ValidJson<CreateAnnouncementRequest>,
) -> Result<(StatusCode, Json<Announcement>), ApiError> {
    let mut tx = state.db.begin().await.map_err(TeachingError::from)?;
    let published =
        service::publish_announcement(&mut tx, &actor, id, &body.title, &body.body).await?;
    tx.commit().await.map_err(TeachingError::from)?;
    Ok((StatusCode::CREATED, Json(api_announcement(published))))
}

/// Changes an announcement. Staff and admins. The plugins are not told.
#[utoipa::path(
    patch,
    path = "/announcements/{id}",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Announcement id")),
    request_body = UpdateAnnouncementRequest,
    responses(
        (status = 200, description = "Changed", body = Announcement),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not write content in this course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such announcement, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The course is archived", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid title or text", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn update_announcement(
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(body): ValidJson<UpdateAnnouncementRequest>,
) -> Result<Json<Announcement>, ApiError> {
    let mut tx = state.db.begin().await.map_err(TeachingError::from)?;
    let changed = service::update_announcement(
        &mut tx,
        &actor,
        id,
        body.title.as_deref(),
        body.body.as_deref(),
    )
    .await?;
    tx.commit().await.map_err(TeachingError::from)?;
    Ok(Json(api_announcement(changed)))
}

/// Deletes an announcement. Staff and admins.
#[utoipa::path(
    delete,
    path = "/announcements/{id}",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Announcement id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not write content in this course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such announcement, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The course is archived", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn delete_announcement(
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(TeachingError::from)?;
    service::delete_announcement(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(TeachingError::from)?;
    Ok(StatusCode::NO_CONTENT)
}
