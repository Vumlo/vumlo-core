use api_types::{
    GoogleCallbackRequest, GoogleStartRequest, GoogleStartResponse, LoginResponse, ProblemDetails,
};
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{AppendHeaders, IntoResponse, Response};
use axum_extra::extract::CookieJar;

use super::auth::{COOKIE_MAX_AGE_SECS, session_cookie};
use super::extract::check_origin;
use super::{api_user, no_store};
use crate::app::AppState;
use crate::error::{ApiError, ValidJson};
use crate::identity::model::{BrowserBinding, NewSession, SessionKind};
use crate::identity::service::{self, IdentityError};

/// Ties a started sign-in to its browser (ADR-0047). It is sent to the Google endpoints only.
const BINDING_COOKIE: &str = "vumlo_oauth";
const BINDING_PATH: &str = "/api/v1/auth/google";
const BINDING_MAX_AGE_SECS: u64 = 10 * 60;

fn binding_cookie(value: &str, max_age_secs: u64, secure: bool) -> Result<HeaderValue, ApiError> {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{BINDING_COOKIE}={value}; Max-Age={max_age_secs}; Path={BINDING_PATH}; HttpOnly; SameSite=Lax{secure}"
    ))
    .map_err(|_| ApiError::Internal)
}

/// Starts sign-in with Google: answers the URL to send the browser to and sets a cookie that ties
/// the attempt to this browser. Off (`404`) without `[auth.google]`.
#[utoipa::path(
    post,
    path = "/auth/google/start",
    tag = "auth",
    request_body = GoogleStartRequest,
    responses(
        (status = 200, description = "Where to send the browser", body = GoogleStartResponse),
        (status = 404, description = "Sign-in with Google is not configured", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 429, description = "Too many requests", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 503, description = "Google cannot be reached", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn start(
    State(state): State<AppState>,
    ValidJson(request): ValidJson<GoogleStartRequest>,
) -> Result<Response, ApiError> {
    let google = state.google.as_ref().ok_or(ApiError::NotFound)?;
    let begin = google
        .begin(request.login_hint.as_deref())
        .await
        .map_err(IdentityError::from)?;
    let mut tx = state.db.begin().await.map_err(IdentityError::from)?;
    let binding = service::start_google_sign_in(&mut tx, &begin).await?;
    tx.commit().await.map_err(IdentityError::from)?;
    let cookie = binding_cookie(
        binding.expose(),
        BINDING_MAX_AGE_SECS,
        state.config.auth.cookie_secure,
    )?;
    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie), no_store()],
        Json(GoogleStartResponse {
            authorization_url: begin.authorization_url,
        }),
    )
        .into_response())
}

/// Finishes sign-in with Google: checks the state against the browser's cookie, exchanges the
/// code, verifies the ID token and opens a web session, like a password login with `client: web`.
///
/// The Google account needs a verified email in one of `auth.allowed_domains`. A first sign-in
/// creates a `student`; an existing account with that address is linked.
#[utoipa::path(
    post,
    path = "/auth/google/callback",
    tag = "auth",
    request_body = GoogleCallbackRequest,
    responses(
        (status = 200, description = "Signed in; the session cookie is set", body = LoginResponse),
        (status = 401, description = "The attempt is unknown, used, expired or from another \
            browser, or Google did not accept it", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "A different origin, a Google account that may not sign in \
            here, or a disabled account", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "Sign-in with Google is not configured", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 429, description = "Too many requests", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 503, description = "Google cannot be reached", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    ValidJson(request): ValidJson<GoogleCallbackRequest>,
) -> Result<Response, ApiError> {
    let google = state.google.as_ref().ok_or(ApiError::NotFound)?;
    check_origin(&Method::POST, &headers, &state.config.server)?;
    let binding = CookieJar::from_headers(&headers)
        .get(BINDING_COOKIE)
        .and_then(|cookie| BrowserBinding::from_presented(cookie.value()));
    // The attempt is used up here, committed, so a failure later cannot be retried with it.
    let attempt = {
        let mut tx = state.db.begin().await.map_err(IdentityError::from)?;
        let attempt =
            service::take_google_attempt(&mut tx, &request.state, binding.as_ref()).await?;
        tx.commit().await.map_err(IdentityError::from)?;
        attempt
    };
    let profile = google
        .finish(request.code, attempt.nonce, attempt.pkce_verifier)
        .await
        .map_err(IdentityError::from)?;
    let mut tx = state.db.begin().await.map_err(IdentityError::from)?;
    let user =
        service::sign_in_with_google(&mut tx, &profile, &state.config.auth.allowed_domains).await?;
    let NewSession { token, .. } =
        service::create_session_for(&mut tx, &user, SessionKind::Web).await?;
    tx.commit().await.map_err(IdentityError::from)?;

    let secure = state.config.auth.cookie_secure;
    let session = session_cookie(token.expose(), COOKIE_MAX_AGE_SECS, secure)?;
    let forget = binding_cookie("", 0, secure)?;
    Ok((
        StatusCode::OK,
        AppendHeaders([(header::SET_COOKIE, session), (header::SET_COOKIE, forget)]),
        [no_store()],
        Json(LoginResponse::Web(api_user(user))),
    )
        .into_response())
}
