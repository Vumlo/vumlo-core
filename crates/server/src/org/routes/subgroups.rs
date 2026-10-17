use api_types::{
    AddMembersRequest, AddMembersResponse, GroupMember, NameRequest, Page, PageQuery,
    ProblemDetails, Subgroup,
};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use uuid::Uuid;

use super::{api_member, api_page, page_params};
use crate::actor::Actor;
use crate::app::AppState;
use crate::error::{ApiError, ValidJson, ValidQuery};
use crate::org::model;
use crate::org::service::{self, OrgError};

fn api_subgroup(subgroup: model::Subgroup) -> Subgroup {
    Subgroup {
        id: subgroup.id,
        group_id: subgroup.group_id,
        name: subgroup.name,
    }
}

/// Creates a subgroup of a group. Admin only.
#[utoipa::path(
    post,
    path = "/groups/{id}/subgroups",
    tag = "groups",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Group id")),
    request_body = NameRequest,
    responses(
        (status = 201, description = "The new subgroup", body = Subgroup),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such group", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 409, description = "The name is taken in this group", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid name", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn create_subgroup(
    State(state): State<AppState>,
    actor: Actor,
    Path(group_id): Path<Uuid>,
    ValidJson(request): ValidJson<NameRequest>,
) -> Result<(StatusCode, Json<Subgroup>), ApiError> {
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    let subgroup = service::create_subgroup(&mut tx, &actor, group_id, &request.name).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok((StatusCode::CREATED, Json(api_subgroup(subgroup))))
}

/// The subgroups of a group, in creation order. Admin only.
#[utoipa::path(
    get,
    path = "/groups/{id}/subgroups",
    tag = "groups",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Group id"), PageQuery),
    responses(
        (status = 200, description = "A page of subgroups", body = Page<Subgroup>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such group", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn list_subgroups(
    State(state): State<AppState>,
    actor: Actor,
    Path(group_id): Path<Uuid>,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<Page<Subgroup>>, ApiError> {
    let page = page_params(&query)?;
    let mut conn = state.db.acquire().await.map_err(OrgError::from)?;
    let subgroups = service::list_subgroups(&mut conn, &actor, group_id, page).await?;
    Ok(Json(api_page(subgroups, api_subgroup)))
}

/// Renames a subgroup. Admin only.
#[utoipa::path(
    patch,
    path = "/subgroups/{id}",
    tag = "groups",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Subgroup id")),
    request_body = NameRequest,
    responses(
        (status = 200, description = "The renamed subgroup", body = Subgroup),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such subgroup", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 409, description = "The name is taken in this group", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid name", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn rename_subgroup(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<NameRequest>,
) -> Result<Json<Subgroup>, ApiError> {
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    let subgroup = service::rename_subgroup(&mut tx, &actor, id, &request.name).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok(Json(api_subgroup(subgroup)))
}

/// Deletes a subgroup with its members. Admin only.
#[utoipa::path(
    delete,
    path = "/subgroups/{id}",
    tag = "groups",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Subgroup id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such subgroup", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 409, description = "A course is linked to it", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn delete_subgroup(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    service::delete_subgroup(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The members of a subgroup by surname, name and id; the cursor is a user id. Admin only.
#[utoipa::path(
    get,
    path = "/subgroups/{id}/members",
    tag = "groups",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Subgroup id"), PageQuery),
    responses(
        (status = 200, description = "A page of members", body = Page<GroupMember>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such subgroup", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn list_subgroup_members(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<Page<GroupMember>>, ApiError> {
    let page = page_params(&query)?;
    let mut conn = state.db.acquire().await.map_err(OrgError::from)?;
    let members = service::list_subgroup_members(&mut conn, &actor, id, page).await?;
    Ok(Json(api_page(members, api_member)))
}

/// Adds members of the group to one of its subgroups. Admin only.
///
/// All or nothing, and idempotent. An entry that is not a member of the group is a `422` on
/// `user_ids[i]`.
#[utoipa::path(
    post,
    path = "/subgroups/{id}/members",
    tag = "groups",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Subgroup id")),
    request_body = AddMembersRequest,
    responses(
        (status = 200, description = "The users who were added", body = AddMembersResponse),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such subgroup", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "A user who is not a member of the group",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn add_subgroup_members(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<AddMembersRequest>,
) -> Result<Json<AddMembersResponse>, ApiError> {
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    let added = service::add_subgroup_members(&mut tx, &actor, id, &request.user_ids).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok(Json(AddMembersResponse { added }))
}

/// Removes a user from a subgroup; they stay in the group. Admin only.
#[utoipa::path(
    delete,
    path = "/subgroups/{id}/members/{user_id}",
    tag = "groups",
    security(("cookie" = []), ("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Subgroup id"),
        ("user_id" = Uuid, Path, description = "The member to remove"),
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such subgroup or member", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn remove_subgroup_member(
    State(state): State<AppState>,
    actor: Actor,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    service::remove_subgroup_member(&mut tx, &actor, id, user_id).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok(StatusCode::NO_CONTENT)
}
