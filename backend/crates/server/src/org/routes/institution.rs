use api_types::{AdminInstitution, Institution, ProblemDetails};
use axum::Json;
use axum::extract::State;

use crate::actor::Actor;
use crate::app::AppState;
use crate::error::ApiError;
use crate::org::service;
use vumlo_config::{self as settings, InstitutionSettings};

fn api_institution(institution: &InstitutionSettings) -> Institution {
    Institution {
        name: institution.name.clone(),
        short_name: institution.short_name.clone(),
        language: match institution.language {
            settings::Language::En => api_types::Language::En,
            settings::Language::Uk => api_types::Language::Uk,
        },
        time_zone: institution.time_zone.name().to_owned(),
        grading_scale: match institution.grading_scale {
            settings::GradingScale::Twelve => api_types::GradingScale::Twelve,
            settings::GradingScale::Hundred => api_types::GradingScale::Hundred,
        },
    }
}

/// The public profile of the institution. No session needed: the sign-in page shows it.
#[utoipa::path(
    get,
    path = "/institution",
    tag = "institution",
    responses((status = 200, description = "The institution profile", body = Institution))
)]
pub async fn get_institution(State(state): State<AppState>) -> Json<Institution> {
    Json(api_institution(&state.config.institution))
}

/// The institution profile with the settings that are not public. Admin only, read-only: they
/// change in the configuration file (ADR-0044).
#[utoipa::path(
    get,
    path = "/admin/institution",
    tag = "institution",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The institution settings", body = AdminInstitution),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn get_admin_institution(
    State(state): State<AppState>,
    actor: Actor,
) -> Result<Json<AdminInstitution>, ApiError> {
    service::view_institution_settings(&actor)?;
    let institution = &state.config.institution;
    Ok(Json(AdminInstitution {
        institution: api_institution(institution),
        mode: match institution.mode {
            settings::InstitutionMode::School => api_types::InstitutionMode::School,
            settings::InstitutionMode::University => api_types::InstitutionMode::University,
        },
        allowed_domains: state.config.auth.allowed_domains.clone(),
    }))
}
