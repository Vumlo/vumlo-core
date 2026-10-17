//! Group links, the group options and exclusions of a course (ADR-0033).

use api_types::{
    CourseGroupLink, Exclusion, GroupOption, LinkGroupRequest, Page, PageQuery, ProblemDetails,
    SubgroupOption,
};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use uuid::Uuid;

use super::page_params;
use crate::actor::Actor;
use crate::app::AppState;
use crate::error::{ApiError, ValidJson, ValidQuery};
use crate::org;
use crate::teaching::model;
use crate::teaching::service::{self, TeachingError};

fn api_link(link: model::CourseGroupLink) -> CourseGroupLink {
    CourseGroupLink {
        id: link.id,
        course_id: link.course_id,
        group_id: link.group_id,
        subgroup_id: link.subgroup_id,
        created_at: link.created_at,
    }
}

fn api_option(option: org::GroupOption) -> GroupOption {
    GroupOption {
        id: option.id,
        name: option.name,
        subgroups: option
            .subgroups
            .into_iter()
            .map(|subgroup| SubgroupOption {
                id: subgroup.id,
                name: subgroup.name,
            })
            .collect(),
    }
}

fn api_exclusion(exclusion: model::Exclusion) -> Exclusion {
    Exclusion {
        user_id: exclusion.user_id,
        created_by: exclusion.created_by,
        created_at: exclusion.created_at,
    }
}

/// The groups and subgroups linked to a course, in link-id order. Assistants, teachers, owners
/// and admins.
#[utoipa::path(
    get,
    path = "/courses/{id}/groups",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id"), PageQuery),
    responses(
        (status = 200, description = "A page of links", body = Page<CourseGroupLink>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not manage the roster",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn list_links(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<Page<CourseGroupLink>>, ApiError> {
    let page = page_params(&query)?;
    let mut conn = state.db.acquire().await.map_err(TeachingError::from)?;
    let links = service::list_links(&mut conn, &actor, id, page).await?;
    Ok(Json(Page {
        items: links.items.into_iter().map(api_link).collect(),
        next_cursor: links.next.map(|id| id.to_string()),
    }))
}

/// Links the course to a group, or to one subgroup of it. Assistants, teachers, owners, admins.
///
/// Live: the students of the group are students of the course while the link exists.
/// Idempotent: a link that exists is returned with `200` instead of `201`.
#[utoipa::path(
    post,
    path = "/courses/{id}/groups",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    request_body = LinkGroupRequest,
    responses(
        (status = 201, description = "The new link", body = CourseGroupLink),
        (status = 200, description = "The link existed already", body = CourseGroupLink),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not manage the roster",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "No such group, or no such subgroup in that group",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn link_group(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<LinkGroupRequest>,
) -> Result<(StatusCode, Json<CourseGroupLink>), ApiError> {
    let mut tx = state.db.begin().await.map_err(TeachingError::from)?;
    let (link, created) =
        service::link_group(&mut tx, &actor, id, request.group_id, request.subgroup_id).await?;
    tx.commit().await.map_err(TeachingError::from)?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(api_link(link))))
}

/// Removes a link. Its students leave the roster unless they have a direct membership or another
/// link; an exclusion of theirs stays as it is.
#[utoipa::path(
    delete,
    path = "/courses/{id}/groups/{link_id}",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Course id"),
        ("link_id" = Uuid, Path, description = "The link to remove"),
    ),
    responses(
        (status = 204, description = "Unlinked"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not manage the roster",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course or link", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn unlink_group(
    State(state): State<AppState>,
    actor: Actor,
    Path((id, link_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(TeachingError::from)?;
    service::unlink_group(&mut tx, &actor, id, link_id).await?;
    tx.commit().await.map_err(TeachingError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The groups a course can be linked to, each with its subgroups: ids and names only, in group-id
/// order. For those who may manage the roster; `org`'s own group endpoints stay admin only.
#[utoipa::path(
    get,
    path = "/courses/{id}/group-options",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id"), PageQuery),
    responses(
        (status = 200, description = "A page of groups", body = Page<GroupOption>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not manage the roster",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn group_options(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<Page<GroupOption>>, ApiError> {
    let page = page_params(&query)?;
    let mut conn = state.db.acquire().await.map_err(TeachingError::from)?;
    let options = service::group_options(&mut conn, &actor, id, page).await?;
    Ok(Json(Page {
        items: options.items.into_iter().map(api_option).collect(),
        next_cursor: options.next.map(|id| id.to_string()),
    }))
}

/// The people taken off the roster of this course, ordered by user id.
///
/// An exclusion stays until it is restored, also when the person has left the group since.
#[utoipa::path(
    get,
    path = "/courses/{id}/exclusions",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id"), PageQuery),
    responses(
        (status = 200, description = "A page of exclusions", body = Page<Exclusion>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not manage the roster",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn list_exclusions(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<Page<Exclusion>>, ApiError> {
    let page = page_params(&query)?;
    let mut conn = state.db.acquire().await.map_err(TeachingError::from)?;
    let exclusions = service::list_exclusions(&mut conn, &actor, id, page).await?;
    Ok(Json(Page {
        items: exclusions.items.into_iter().map(api_exclusion).collect(),
        next_cursor: exclusions.next.map(|id| id.to_string()),
    }))
}

/// Deletes an exclusion: if a linked group still has the person, they are on the roster again.
/// Grades and submissions were never touched.
#[utoipa::path(
    delete,
    path = "/courses/{id}/exclusions/{user_id}",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Course id"),
        ("user_id" = Uuid, Path, description = "The excluded user"),
    ),
    responses(
        (status = 204, description = "Restored"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not manage the roster",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course or exclusion", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn restore_exclusion(
    State(state): State<AppState>,
    actor: Actor,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(TeachingError::from)?;
    service::restore_exclusion(&mut tx, &actor, id, user_id).await?;
    tx.commit().await.map_err(TeachingError::from)?;
    Ok(StatusCode::NO_CONTENT)
}
