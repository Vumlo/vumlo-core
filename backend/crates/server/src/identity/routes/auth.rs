use api_types::{
    AppLoginResponse, LoginClient, LoginRequest, LoginResponse, PasswordResetConfirmRequest,
    PasswordResetRequest, ProblemDetails, RegisterRequest, VerifyEmailRequest,
};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use std::sync::Arc;

use super::extract::check_origin;
use super::{COOKIE_NAME, MailThrottle, api_user, no_store, password_error, session_kind};
use crate::actor::PendingActor;
use crate::app::AppState;
use crate::error::{ApiError, ValidJson};
use crate::identity::model::{EmailToken, ModelError, NewSession};
use crate::identity::service::{
    self, IdentityError, LoginThrottle, RegistrationRules, ThrottleKey,
};

/// The browsers' maximum cookie lifetime. The session's own `expires_at` decides validity, so
/// a sliding session is not cut short by the cookie.
pub(super) const COOKIE_MAX_AGE_SECS: u64 = 400 * 24 * 60 * 60;

pub(super) fn session_cookie(
    token: &str,
    max_age_secs: u64,
    secure: bool,
) -> Result<HeaderValue, ApiError> {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{COOKIE_NAME}={token}; Max-Age={max_age_secs}; Path=/; HttpOnly; SameSite=Lax{secure}"
    ))
    .map_err(|_| ApiError::Internal)
}

/// Logs in with email and password.
///
/// `client = web` sets the `vumlo_session` cookie and returns the user. `client = app`
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
        check_origin(&Method::POST, &headers, &state.config.server)?;
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

/// Counts the request against the address, whether or not it has an account.
fn throttle_mail(throttle: &MailThrottle, email: &str) -> Result<(), ApiError> {
    Ok(throttle.0.reserve(&ThrottleKey::new(email))?)
}

/// Creates an account for the address and mails a link that verifies it.
///
/// The answer is the same for every address, so a page cannot tell who has an account (ADR-0046).
/// The account cannot log in until the link is used. Off (`404`) without `[mail]`.
#[utoipa::path(
    post,
    path = "/auth/register",
    tag = "auth",
    request_body = RegisterRequest,
    responses(
        (status = 202, description = "Accepted; if the address can sign up, a mail is on its way"),
        (status = 404, description = "Self-registration is off: the server has no mail configured",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "An address outside the allowed domains, or a password that \
            breaks the policy", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "Too many requests", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn register(
    State(state): State<AppState>,
    Extension(throttle): Extension<Arc<MailThrottle>>,
    ValidJson(request): ValidJson<RegisterRequest>,
) -> Result<StatusCode, ApiError> {
    if state.mailer.is_none() {
        return Err(ApiError::NotFound);
    }
    throttle_mail(&throttle, &request.email)?;
    let rules = RegistrationRules {
        allowed_domains: &state.config.auth.allowed_domains,
        password_min_length: state.config.auth.password_min_length,
        institution_name: &state.config.institution.name,
    };
    let prepared = service::prepare_registration(
        &state.hashing,
        rules,
        &request.name,
        &request.surname,
        &request.email,
        request.password,
    )
    .await
    .map_err(|error| match error {
        IdentityError::Invalid(ModelError::Password(reasons)) => {
            password_error("password", reasons)
        }
        other => other.into(),
    })?;
    let mut tx = state.db.begin().await.map_err(IdentityError::from)?;
    service::register(&mut tx, prepared).await?;
    tx.commit().await.map_err(IdentityError::from)?;
    Ok(StatusCode::ACCEPTED)
}

/// Verifies the address with the token from the mail. Does not sign anyone in.
#[utoipa::path(
    post,
    path = "/auth/verify-email",
    tag = "auth",
    request_body = VerifyEmailRequest,
    responses(
        (status = 204, description = "The address is verified; the account can log in"),
        (status = 410, description = "The link is unknown, used or expired", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 429, description = "Too many requests", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn verify_email(
    State(state): State<AppState>,
    ValidJson(request): ValidJson<VerifyEmailRequest>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(IdentityError::from)?;
    service::verify_email(&mut tx, EmailToken::from_presented(&request.token)).await?;
    tx.commit().await.map_err(IdentityError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Mails a link that lets the owner of an account choose a new password.
///
/// The answer is the same for every address (ADR-0046). Off (`404`) without `[mail]`.
#[utoipa::path(
    post,
    path = "/auth/password-reset",
    tag = "auth",
    request_body = PasswordResetRequest,
    responses(
        (status = 202, description = "Accepted; if the address has an account, a mail is on its way"),
        (status = 404, description = "Password reset is off: the server has no mail configured",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "Too many requests", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn request_password_reset(
    State(state): State<AppState>,
    Extension(throttle): Extension<Arc<MailThrottle>>,
    ValidJson(request): ValidJson<PasswordResetRequest>,
) -> Result<StatusCode, ApiError> {
    if state.mailer.is_none() {
        return Err(ApiError::NotFound);
    }
    throttle_mail(&throttle, &request.email)?;
    let mut tx = state.db.begin().await.map_err(IdentityError::from)?;
    service::request_password_reset(&mut tx, &request.email).await?;
    tx.commit().await.map_err(IdentityError::from)?;
    Ok(StatusCode::ACCEPTED)
}

/// Sets a new password with the token from the reset mail.
///
/// Revokes every session of the user. A spent or expired link is `410` and is decided before the
/// password is looked at.
#[utoipa::path(
    post,
    path = "/auth/password-reset/confirm",
    tag = "auth",
    request_body = PasswordResetConfirmRequest,
    responses(
        (status = 204, description = "The password is set; the user can log in with it"),
        (status = 410, description = "The link is unknown, used or expired", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "A password that breaks the policy; `errors` explains why",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "Too many requests", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn confirm_password_reset(
    State(state): State<AppState>,
    ValidJson(request): ValidJson<PasswordResetConfirmRequest>,
) -> Result<StatusCode, ApiError> {
    let token = EmailToken::from_presented(&request.token);
    let target = {
        let mut conn = state.db.acquire().await.map_err(IdentityError::from)?;
        service::check_reset_token(&mut conn, token.as_ref()).await?
    };
    let prepared = service::prepare_password_reset(
        &state.hashing,
        target,
        request.new_password,
        state.config.auth.password_min_length,
        &state.config.institution.name,
    )
    .await?;
    let mut tx = state.db.begin().await.map_err(IdentityError::from)?;
    service::confirm_password_reset(&mut tx, token.as_ref(), prepared).await?;
    tx.commit().await.map_err(IdentityError::from)?;
    Ok(StatusCode::NO_CONTENT)
}
