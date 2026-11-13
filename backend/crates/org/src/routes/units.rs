use api_types::{
    CreateUnitRequest, Page, ProblemDetails, Unit, UnitAbility, UnitSummary, UpdateUnitRequest,
};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use uuid::Uuid;

use crate::OrgState;
use crate::model::{self, OrgName, UnitUpdate};
use crate::service::{self, OrgError};
use access::{self, Actor, UnitGrant};
use common::{ApiError, ValidJson};

fn api_unit(unit: model::Unit) -> Unit {
    Unit {
        id: unit.id,
        parent_id: unit.parent_id,
        name: unit.name,
        created_at: unit.created_at,
    }
}

fn api_ability(ability: access::UnitAbility) -> UnitAbility {
    match ability {
        access::UnitAbility::ViewGrades => UnitAbility::ViewGrades,
        access::UnitAbility::EditContent => UnitAbility::EditContent,
        access::UnitAbility::Grade => UnitAbility::Grade,
        access::UnitAbility::ManageRoster => UnitAbility::ManageRoster,
        access::UnitAbility::ManageCourses => UnitAbility::ManageCourses,
        access::UnitAbility::ManageUnits => UnitAbility::ManageUnits,
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
        (status = 404, description = "No such parent, or the caller cannot see it",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The name is taken under the parent", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid name", body = ProblemDetails,
            content_type = "application/problem+json"),
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
        (status = 404, description = "No such unit (or new parent), or the caller cannot see it",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The name is taken under the parent", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid name, or the new parent is the unit or below it",
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
