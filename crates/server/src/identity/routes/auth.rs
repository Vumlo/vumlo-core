use api_types::{AppLoginResponse, LoginClient, LoginRequest, LoginResponse, ProblemDetails};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use std::sync::Arc;

use super::extract::check_origin;
use super::{COOKIE_NAME, api_user, no_store, session_kind};
use crate::actor::PendingActor;
use crate::app::AppState;
use crate::error::{ApiError, ValidJson};
use crate::identity::model::NewSession;
use crate::identity::service::{self, IdentityError, LoginThrottle};

/// The browsers' maximum cookie lifetime. The session's own `expires_at` decides validity, so
/// a sliding session is not cut short by the cookie.
const COOKIE_MAX_AGE_SECS: u64 = 400 * 24 * 60 * 60;

fn session_cookie(token: &str, max_age_secs: u64, secure: bool) -> Result<HeaderValue, ApiError> {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{COOKIE_NAME}={token}; Max-Age={max_age_secs}; Path=/; HttpOnly; SameSite=Lax{secure}"
    ))
    .map_err(|_| ApiError::Internal)
}

/// Logs in with email and password.
///
/// `client = web` sets the `learnly_session` cookie and returns the user. `client = app`
/// returns the token, to be sent as `Authorization: Bearer`. Failures never say whether the
/// email or the password was wrong.
#[utoipa::path(
    post,
    path = "/auth/login",
    tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Logged in", body = LoginResponse),
        (status = 401, description = "Wrong email or password", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Web login from a different origin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Malformed request", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 429, description = "Too many attempts", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn login(
    State(state): State<AppState>,
    Extension(throttle): Extension<Arc<LoginThrottle>>,
    headers: HeaderMap,
    ValidJson(request): ValidJson<LoginRequest>,
) -> Result<Response, ApiError> {
    if request.client == LoginClient::Web {
        // A login with no session yet is still a state change: refuse cross-site form posts.
        check_origin(&Method::POST, &headers, &state.config.server.public_origin)?;
    }
    let attempt = {
        let mut conn = state.db.acquire().await.map_err(IdentityError::from)?;
        service::lookup_login(&mut conn, &throttle, &request.email).await?
    };
    let login =
        service::complete_login(&throttle, &state.hashing, attempt, &request.password).await?;

    let kind = session_kind(request.client);
    let mut tx = state.db.begin().await.map_err(IdentityError::from)?;
    let session = service::create_session(&mut tx, &login, kind).await?;
    tx.commit().await.map_err(IdentityError::from)?;

    let NewSession {
        token, expires_at, ..
    } = session;
    let user = api_user(login.user);
    let response = match request.client {
        LoginClient::Web => {
            let cookie = session_cookie(
                token.expose(),
                COOKIE_MAX_AGE_SECS,
                state.config.auth.cookie_secure,
            )?;
            (
                StatusCode::OK,
                [(header::SET_COOKIE, cookie), no_store()],
                Json(LoginResponse::Web(user)),
            )
                .into_response()
        }
        LoginClient::App => (
            StatusCode::OK,
            [no_store()],
            Json(LoginResponse::App(AppLoginResponse {
                token: token.expose().to_owned(),
                expires_at,
                user,
            })),
        )
            .into_response(),
    };
    Ok(response)
}

/// Ends the current session and clears the cookie.
#[utoipa::path(
    post,
    path = "/auth/logout",
    tag = "auth",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 204, description = "Logged out"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Cookie request from a different origin", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn logout(
    State(state): State<AppState>,
    PendingActor(actor): PendingActor,
) -> Result<Response, ApiError> {
    let mut tx = state.db.begin().await.map_err(IdentityError::from)?;
    service::logout(&mut tx, &actor).await?;
    tx.commit().await.map_err(IdentityError::from)?;
    let clear = session_cookie("", 0, state.config.auth.cookie_secure)?;
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, clear)]).into_response())
}
