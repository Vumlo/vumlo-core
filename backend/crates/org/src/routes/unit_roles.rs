//! What each unit role may do, switched by an administrator (ADR-0094).

use api_types::{
    ProblemDetails, SetUnitRoleAbilitiesRequest, UnitAbility, UnitRole, UnitRoleAbilities,
};
use axum::Json;
use axum::extract::{Path, State};

use super::units::{api_ability, domain_ability, domain_role};
use crate::OrgState;
use crate::service::{self, OrgError};
use access::{self, Actor};
use common::{ApiError, ValidJson};

fn api_abilities(all: Vec<(access::UnitRole, Vec<access::UnitAbility>)>) -> UnitRoleAbilities {
    let of = |wanted: access::UnitRole| -> Vec<UnitAbility> {
        all.iter()
            .filter(|(role, _)| *role == wanted)
            .flat_map(|(_, abilities)| abilities.iter().copied().map(api_ability))
            .collect()
    };
    UnitRoleAbilities {
        manager: of(access::UnitRole::Manager),
        viewer: of(access::UnitRole::Viewer),
    }
}

/// The switches of each unit role, as they stand. Administrators only.
#[utoipa::path(
    get,
    path = "/admin/unit-roles",
    tag = "units",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The switches of each role", body = UnitRoleAbilities),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn get_unit_role_abilities(
    State(state): State<OrgState>,
    actor: Actor,
) -> Result<Json<UnitRoleAbilities>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(OrgError::from)?;
    let all = service::unit_role_abilities(&mut conn, &actor).await?;
    Ok(Json(api_abilities(all)))
}

/// Replaces the switches of a unit role by exactly these (viewing is implicit and has none). It takes
/// effect on the next request of everyone who holds the role. Administrators only.
#[utoipa::path(
    put,
    path = "/admin/unit-roles/{role}",
    tag = "units",
    security(("cookie" = []), ("bearer" = [])),
    params(("role" = UnitRole, Path, description = "The unit role")),
    request_body = SetUnitRoleAbilitiesRequest,
    responses(
        (status = 200, description = "The switches of each role afterwards",
            body = UnitRoleAbilities),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "An unknown ability", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn put_unit_role_abilities(
    State(state): State<OrgState>,
    actor: Actor,
    Path(role): Path<UnitRole>,
    ValidJson(request): ValidJson<SetUnitRoleAbilitiesRequest>,
) -> Result<Json<UnitRoleAbilities>, ApiError> {
    let abilities = request.abilities.into_iter().map(domain_ability).collect();
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    service::set_unit_role_abilities(&mut tx, &actor, domain_role(role), abilities).await?;
    tx.commit().await.map_err(OrgError::from)?;
    let mut conn = state.db.acquire().await.map_err(OrgError::from)?;
    let all = service::unit_role_abilities(&mut conn, &actor).await?;
    Ok(Json(api_abilities(all)))
}
