use api_types::{
    AddMembersRequest, AddMembersResponse, Group, GroupMember, NameRequest, Page, PageQuery,
    ProblemDetails, RemoveMembersRequest, RemoveMembersResponse, ReplaceMembersRequest,
    ReplaceMembersResponse,
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

fn api_group(group: model::Group) -> Group {
    Group {
        id: group.id,
        name: group.name,
        created_at: group.created_at,
    }
}

/// Creates a group (a cohort). Admin only.
#[utoipa::path(
    post,
    path = "/groups",
    tag = "groups",
    security(("cookie" = []), ("bearer" = [])),
    request_body = NameRequest,
    responses(
        (status = 201, description = "The new group", body = Group),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 409, description = "The name is taken", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid name", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn create_group(
    State(state): State<AppState>,
    actor: Actor,
    ValidJson(request): ValidJson<NameRequest>,
) -> Result<(StatusCode, Json<Group>), ApiError> {
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    let group = service::create_group(&mut tx, &actor, &request.name).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok((StatusCode::CREATED, Json(api_group(group))))
}

/// The groups, in creation order. Admin only for now (ADR-0032).
#[utoipa::path(
    get,
    path = "/groups",
    tag = "groups",
    security(("cookie" = []), ("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "A page of groups", body = Page<Group>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn list_groups(
    State(state): State<AppState>,
    actor: Actor,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<Page<Group>>, ApiError> {
    let page = page_params(&query)?;
    let mut conn = state.db.acquire().await.map_err(OrgError::from)?;
    let groups = service::list_groups(&mut conn, &actor, page).await?;
    Ok(Json(api_page(groups, api_group)))
}

/// One group. Admin only.
#[utoipa::path(
    get,
    path = "/groups/{id}",
    tag = "groups",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Group id")),
    responses(
        (status = 200, description = "The group", body = Group),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such group", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn get_group(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<Group>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(OrgError::from)?;
    Ok(Json(api_group(
        service::get_group(&mut conn, &actor, id).await?,
    )))
}

/// Renames a group. Admin only.
#[utoipa::path(
    patch,
    path = "/groups/{id}",
    tag = "groups",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Group id")),
    request_body = NameRequest,
    responses(
        (status = 200, description = "The renamed group", body = Group),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such group", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 409, description = "The name is taken", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid name", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn rename_group(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<NameRequest>,
) -> Result<Json<Group>, ApiError> {
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    let group = service::rename_group(&mut tx, &actor, id, &request.name).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok(Json(api_group(group)))
}

/// Deletes a group with its subgroups and members. Admin only.
#[utoipa::path(
    delete,
    path = "/groups/{id}",
    tag = "groups",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Group id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such group", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 409, description = "A course is linked to it", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn delete_group(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    service::delete_group(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The members of a group by surname, name and id; the cursor is a user id. Admin only.
#[utoipa::path(
    get,
    path = "/groups/{id}/members",
    tag = "groups",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Group id"), PageQuery),
    responses(
        (status = 200, description = "A page of members", body = Page<GroupMember>),
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
pub async fn list_group_members(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<Page<GroupMember>>, ApiError> {
    let page = page_params(&query)?;
    let mut conn = state.db.acquire().await.map_err(OrgError::from)?;
    let members = service::list_group_members(&mut conn, &actor, id, page).await?;
    Ok(Json(api_page(members, api_member)))
}

/// Adds students to a group. Admin only.
///
/// All or nothing, and idempotent: users who are members already are skipped and not listed in
/// `added`. An entry that is no user or not a student is a `422` on `user_ids[i]`.
#[utoipa::path(
    post,
    path = "/groups/{id}/members",
    tag = "groups",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Group id")),
    request_body = AddMembersRequest,
    responses(
        (status = 200, description = "The users who were added", body = AddMembersResponse),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such group", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "An unknown user or a user who is not a student",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn add_group_members(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<AddMembersRequest>,
) -> Result<Json<AddMembersResponse>, ApiError> {
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    let added = service::add_group_members(&mut tx, &actor, id, &request.user_ids).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok(Json(AddMembersResponse { added }))
}

/// Replaces the members of a group: the rollover of a class in one call. Admin only.
///
/// The group has exactly `user_ids` afterwards: the ones who were not members are added, the
/// members who are not listed are removed, with their subgroup memberships. All or nothing: an
/// entry that is no user or not a student is a `422` on `user_ids[i]`. Linked courses follow at
/// once; the students who left become former students of them, with their results (ADR-0037).
#[utoipa::path(
    put,
    path = "/groups/{id}/members",
    tag = "groups",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Group id")),
    request_body = ReplaceMembersRequest,
    responses(
        (status = 200, description = "Who was added and who was removed", body = ReplaceMembersResponse),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such group", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "An unknown user or a user who is not a student",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn replace_group_members(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<ReplaceMembersRequest>,
) -> Result<Json<ReplaceMembersResponse>, ApiError> {
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    let (added, removed) =
        service::replace_group_members(&mut tx, &actor, id, &request.user_ids).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok(Json(ReplaceMembersResponse { added, removed }))
}

/// Removes several users from a group and from its subgroups. Admin only.
///
/// Users who are not members are skipped; `removed` lists the ones who were.
#[utoipa::path(
    post,
    path = "/groups/{id}/members/remove",
    tag = "groups",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Group id")),
    request_body = RemoveMembersRequest,
    responses(
        (status = 200, description = "The users who were removed", body = RemoveMembersResponse),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such group", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn remove_group_members(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<RemoveMembersRequest>,
) -> Result<Json<RemoveMembersResponse>, ApiError> {
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    let removed = service::remove_group_members(&mut tx, &actor, id, &request.user_ids).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok(Json(RemoveMembersResponse { removed }))
}

/// Removes a user from a group and from its subgroups. Admin only.
#[utoipa::path(
    delete,
    path = "/groups/{id}/members/{user_id}",
    tag = "groups",
    security(("cookie" = []), ("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Group id"),
        ("user_id" = Uuid, Path, description = "The member to remove"),
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such group or member", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn remove_group_member(
    State(state): State<AppState>,
    actor: Actor,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    service::remove_group_member(&mut tx, &actor, id, user_id).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok(StatusCode::NO_CONTENT)
}
