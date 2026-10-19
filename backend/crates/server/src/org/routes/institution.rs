use api_types::{
    AdminInstitution, AppDownload, AppPlatform, Institution, ProblemDetails, SetAppDownloadsRequest,
};
use axum::Json;
use axum::extract::State;

use crate::actor::Actor;
use crate::app::AppState;
use crate::error::{ApiError, ValidJson};
use crate::org::service::{self, OrgError};
use vumlo_config::{self as settings, InstitutionSettings};

fn platform_of(name: &str) -> Option<AppPlatform> {
    Some(match name {
        "windows" => AppPlatform::Windows,
        "macos_arm" => AppPlatform::MacosArm,
        "macos_intel" => AppPlatform::MacosIntel,
        "linux_deb" => AppPlatform::LinuxDeb,
        "linux_appimage" => AppPlatform::LinuxAppimage,
        _ => return None,
    })
}

fn name_of(platform: AppPlatform) -> &'static str {
    match platform {
        AppPlatform::Windows => "windows",
        AppPlatform::MacosArm => "macos_arm",
        AppPlatform::MacosIntel => "macos_intel",
        AppPlatform::LinuxDeb => "linux_deb",
        AppPlatform::LinuxAppimage => "linux_appimage",
    }
}

fn api_downloads(rows: Vec<(String, String)>) -> Vec<AppDownload> {
    rows.into_iter()
        .filter_map(|(platform, url)| {
            Some(AppDownload {
                platform: platform_of(&platform)?,
                url,
            })
        })
        .collect()
}

fn api_institution(
    institution: &InstitutionSettings,
    app_downloads: Vec<AppDownload>,
) -> Institution {
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
        app_downloads,
    }
}

/// The links to the app, or none: the profile is what the sign-in page shows, and it must not fail
/// because a small list could not be read.
async fn downloads_or_none(state: &AppState) -> Vec<AppDownload> {
    let rows = match state.db.acquire().await {
        Ok(mut conn) => service::app_downloads(&mut conn).await,
        Err(error) => Err(OrgError::from(error)),
    };
    match rows {
        Ok(rows) => api_downloads(rows),
        Err(error) => {
            tracing::warn!(%error, "the app download links could not be read");
            Vec::new()
        }
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
    let downloads = downloads_or_none(&state).await;
    Json(api_institution(&state.config.institution, downloads))
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
    let downloads = downloads_or_none(&state).await;
    Ok(Json(AdminInstitution {
        institution: api_institution(institution, downloads),
        mode: match institution.mode {
            settings::InstitutionMode::School => api_types::InstitutionMode::School,
            settings::InstitutionMode::University => api_types::InstitutionMode::University,
        },
        allowed_domains: state.config.auth.allowed_domains.clone(),
    }))
}

/// Sets the links to the desktop app's installers (ADR-0064): the whole list, so what is missing is removed.
/// Admin only. The web app shows a hint and a download page when the list is not empty.
#[utoipa::path(
    put,
    path = "/admin/institution/app-downloads",
    tag = "institution",
    security(("cookie" = []), ("bearer" = [])),
    request_body = SetAppDownloadsRequest,
    responses(
        (status = 200, description = "The links now set", body = Vec<AppDownload>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "A link that is not an https address, or a platform twice",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn set_app_downloads(
    State(state): State<AppState>,
    actor: Actor,
    ValidJson(request): ValidJson<SetAppDownloadsRequest>,
) -> Result<Json<Vec<AppDownload>>, ApiError> {
    let links: Vec<(&'static str, String)> = request
        .downloads
        .into_iter()
        .map(|download| (name_of(download.platform), download.url))
        .collect();
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    let rows = service::set_app_downloads(&mut tx, &actor, &links).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok(Json(api_downloads(rows)))
}
