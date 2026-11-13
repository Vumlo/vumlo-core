use api_types::{
    Candidate, Candidates, CandidatesQuery, CreateUnitRequest, Page, ProblemDetails,
    SetUnitRoleRequest, Unit, UnitAbility, UnitRole, UnitRoleEntry, UnitSummary, UpdateUnitRequest,
};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use uuid::Uuid;

use crate::OrgState;
use crate::model::{self, OrgName, UnitUpdate};
use crate::service::{self, OrgError};
use access::{self, Actor, UnitGrant};
use common::{ApiError, ValidJson, ValidQuery};

fn api_unit(unit: model::Unit) -> Unit {
    Unit {
        id: unit.id,
        parent_id: unit.parent_id,
        name: unit.name,
        created_at: unit.created_at,
    }
}

pub(super) fn api_ability(ability: access::UnitAbility) -> UnitAbility {
    match ability {
        access::UnitAbility::ViewGrades => UnitAbility::ViewGrades,
        access::UnitAbility::EditContent => UnitAbility::EditContent,
        access::UnitAbility::Grade => UnitAbility::Grade,
        access::UnitAbility::ManageRoster => UnitAbility::ManageRoster,
        access::UnitAbility::ManageCourses => UnitAbility::ManageCourses,
        access::UnitAbility::ManageUnits => UnitAbility::ManageUnits,
    }
}

pub(super) fn domain_ability(ability: UnitAbility) -> access::UnitAbility {
    match ability {
        UnitAbility::ViewGrades => access::UnitAbility::ViewGrades,
        UnitAbility::EditContent => access::UnitAbility::EditContent,
        UnitAbility::Grade => access::UnitAbility::Grade,
        UnitAbility::ManageRoster => access::UnitAbility::ManageRoster,
        UnitAbility::ManageCourses => access::UnitAbility::ManageCourses,
        UnitAbility::ManageUnits => access::UnitAbility::ManageUnits,
    }
}

fn api_summary((unit, grant): (model::Unit, UnitGrant)) -> UnitSummary {
    UnitSummary {
        id: unit.id,
        parent_id: unit.parent_id,
        name: unit.name,
        abilities: grant.abilities().into_iter().map(api_ability).collect(),
    }
}

/// The units the caller sees, flat, by name. An administrator sees all of them with every ability;
/// anyone else sees the units their unit roles cover (an empty list for most people). The whole tree
/// comes in one response, so there is no cursor.
#[utoipa::path(
    get,
    path = "/units",
    tag = "units",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The units the caller sees; `next_cursor` is always null",
            body = Page<UnitSummary>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn list_units(
    State(state): State<OrgState>,
    actor: Actor,
) -> Result<Json<Page<UnitSummary>>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(OrgError::from)?;
    let units = service::list_units(&mut conn, &actor).await?;
    Ok(Json(Page {
        items: units.into_iter().map(api_summary).collect(),
        next_cursor: None,
    }))
}

/// Creates a unit under another one. Needs `manage_units` on the parent; a top-level unit is for
/// administrators.
#[utoipa::path(
    post,
    path = "/units",
    tag = "units",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CreateUnitRequest,
    responses(
        (status = 201, description = "The new unit", body = Unit),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "No `manage_units` on the parent", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 409, description = "The name is taken under the parent", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid name, or a parent_id that names no unit the caller \
            sees", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn create_unit(
    State(state): State<OrgState>,
    actor: Actor,
    ValidJson(request): ValidJson<CreateUnitRequest>,
) -> Result<(StatusCode, Json<Unit>), ApiError> {
    let name = OrgName::parse(&request.name).map_err(OrgError::from)?;
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    let unit = service::create_unit(&mut tx, &actor, name, request.parent_id).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok((StatusCode::CREATED, Json(api_unit(unit))))
}

/// Renames a unit, moves it, or both. Needs `manage_units` on its parent, and on the new parent for
/// a move; a unit cannot move under itself or below.
#[utoipa::path(
    patch,
    path = "/units/{id}",
    tag = "units",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Unit id")),
    request_body = UpdateUnitRequest,
    responses(
        (status = 200, description = "The updated unit", body = Unit),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "No `manage_units` on the parent", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such unit, or the caller cannot see it",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The name is taken under the parent", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid name, a parent_id that names no unit the caller \
            sees, or the new parent is the unit or below it",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn update_unit(
    State(state): State<OrgState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<UpdateUnitRequest>,
) -> Result<Json<Unit>, ApiError> {
    let update = UnitUpdate {
        name: request
            .name
            .as_deref()
            .map(OrgName::parse)
            .transpose()
            .map_err(OrgError::from)?,
        parent_id: request.parent_id,
    };
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    let unit = service::update_unit(&mut tx, &actor, id, update).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok(Json(api_unit(unit)))
}

/// Deletes a unit that has no subunits and no courses. Needs `manage_units` on its parent.
#[utoipa::path(
    delete,
    path = "/units/{id}",
    tag = "units",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Unit id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "No `manage_units` on the parent", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such unit, or the caller cannot see it",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The unit has subunits or courses", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn delete_unit(
    State(state): State<OrgState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    service::delete_unit(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

fn api_role(role: access::UnitRole) -> UnitRole {
    match role {
        access::UnitRole::Manager => UnitRole::Manager,
        access::UnitRole::Viewer => UnitRole::Viewer,
    }
}

pub(super) fn domain_role(role: UnitRole) -> access::UnitRole {
    match role {
        UnitRole::Manager => access::UnitRole::Manager,
        UnitRole::Viewer => access::UnitRole::Viewer,
    }
}

fn api_role_entry(entry: model::UnitRoleEntry) -> UnitRoleEntry {
    UnitRoleEntry {
        user_id: entry.user_id,
        name: entry.name,
        surname: entry.surname,
        email: entry.email,
        role: api_role(entry.role),
    }
}

fn api_candidate(candidate: model::UnitCandidate) -> Candidate {
    Candidate {
        user_id: candidate.user_id,
        name: candidate.name,
        surname: candidate.surname,
        email: candidate.email,
        role: auth::api_role(candidate.role),
    }
}

/// The roles held directly on a unit, by surname. Anyone who sees the unit may list them; the roles
/// of the units above it are not shown. The list is short, so there is no cursor.
#[utoipa::path(
    get,
    path = "/units/{id}/roles",
    tag = "units",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Unit id")),
    responses(
        (status = 200, description = "The roles on this unit; `next_cursor` is always null",
            body = Page<UnitRoleEntry>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such unit, or the caller cannot see it",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn list_unit_roles(
    State(state): State<OrgState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<Page<UnitRoleEntry>>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(OrgError::from)?;
    let entries = service::list_unit_roles(&mut conn, &actor, id).await?;
    Ok(Json(Page {
        items: entries.into_iter().map(api_role_entry).collect(),
        next_cursor: None,
    }))
}

/// Gives a teacher or an administrator a role on the unit, replacing the one they held. Needs
/// `manage_units` on the unit's parent: nobody changes the roles of the unit their own role sits on.
#[utoipa::path(
    put,
    path = "/units/{id}/roles/{user_id}",
    tag = "units",
    security(("cookie" = []), ("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Unit id"),
        ("user_id" = Uuid, Path, description = "The person who gets the role"),
    ),
    request_body = SetUnitRoleRequest,
    responses(
        (status = 204, description = "The person holds the role"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "No `manage_units` on the parent", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such unit (or the caller cannot see it), or no such user",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "The user is a student", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn put_unit_role(
    State(state): State<OrgState>,
    actor: Actor,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
    ValidJson(request): ValidJson<SetUnitRoleRequest>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    service::set_unit_role(&mut tx, &actor, id, user_id, domain_role(request.role)).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Takes a person's role on the unit away. Needs `manage_units` on the unit's parent.
#[utoipa::path(
    delete,
    path = "/units/{id}/roles/{user_id}",
    tag = "units",
    security(("cookie" = []), ("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Unit id"),
        ("user_id" = Uuid, Path, description = "The person who loses the role"),
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "No `manage_units` on the parent", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such unit (or the caller cannot see it), or the person \
            holds no role on it", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn delete_unit_role(
    State(state): State<OrgState>,
    actor: Actor,
    Path((id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    service::remove_unit_role(&mut tx, &actor, id, user_id).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Active teachers and administrators without a role on the unit who match `q` in their name,
/// surname or email: at most 20, ordered by surname. Needs `manage_units` on the unit's parent.
#[utoipa::path(
    get,
    path = "/units/{id}/candidates",
    tag = "units",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Unit id"), CandidatesQuery),
    responses(
        (status = 200, description = "Up to 20 people", body = Candidates),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "No `manage_units` on the parent", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such unit, or the caller cannot see it",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A search text shorter than two characters",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn get_unit_candidates(
    State(state): State<OrgState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidQuery(query): ValidQuery<CandidatesQuery>,
) -> Result<Json<Candidates>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(OrgError::from)?;
    let found = service::unit_candidates(&mut conn, &actor, id, &query.q).await?;
    Ok(Json(Candidates {
        items: found.into_iter().map(api_candidate).collect(),
    }))
}
