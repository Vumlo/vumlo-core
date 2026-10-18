//! Turns a request into an `Actor`: finds the credential, enforces the CSRF rule for cookies,
//! resolves the session and refreshes it at most once an hour.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, header};
use axum_extra::extract::CookieJar;

use crate::actor::{Actor, PendingActor};
use crate::app::AppState;
use crate::error::ApiError;
use crate::identity::model::SessionToken;
use crate::identity::service::{self, Authenticated, IdentityError};
use vumlo_config::ServerSettings;

pub const COOKIE_NAME: &str = "vumlo_session";

enum Credential {
    Bearer(SessionToken),
    Cookie(SessionToken),
}

/// A bearer token wins over a cookie. A credential that is present but malformed is rejected,
/// not ignored.
fn credential(headers: &HeaderMap) -> Result<Credential, IdentityError> {
    if let Some(value) = headers.get(header::AUTHORIZATION) {
        let text = value.to_str().map_err(|_| IdentityError::Unauthenticated)?;
        let (scheme, token) = text.split_once(' ').ok_or(IdentityError::Unauthenticated)?;
        if !scheme.eq_ignore_ascii_case("bearer") {
            return Err(IdentityError::Unauthenticated);
        }
        return SessionToken::from_presented(token.trim())
            .map(Credential::Bearer)
            .ok_or(IdentityError::Unauthenticated);
    }
    let jar = CookieJar::from_headers(headers);
    let cookie = jar.get(COOKIE_NAME).ok_or(IdentityError::Unauthenticated)?;
    SessionToken::from_presented(cookie.value())
        .map(Credential::Cookie)
        .ok_or(IdentityError::Unauthenticated)
}

/// CSRF defence for cookie authentication: a request that is not read-only must come from the
/// public origin or from one of the trusted frontends (ADR-0041). Bearer requests are not sent
/// by browsers on their own, so they skip it.
pub(super) fn check_origin(
    method: &Method,
    headers: &HeaderMap,
    server: &ServerSettings,
) -> Result<(), IdentityError> {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return Ok(());
    }
    match headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        Some(origin)
            if origin == server.public_origin
                || server.allowed_origins.iter().any(|o| o == origin) =>
        {
            Ok(())
        }
        _ => Err(IdentityError::OriginMismatch),
    }
}

async fn authenticate_request(
    parts: &Parts,
    state: &AppState,
) -> Result<Authenticated, IdentityError> {
    let token = match credential(&parts.headers)? {
        Credential::Bearer(token) => token,
        Credential::Cookie(token) => {
            check_origin(&parts.method, &parts.headers, &state.config.server)?;
            token
        }
    };
    let mut conn = state.db.acquire().await?;
    let authenticated = service::authenticate(&mut conn, &token).await?;
    drop(conn);
    if let Some(kind) = authenticated.touch {
        let mut tx = state.db.begin().await?;
        service::touch_session(&mut tx, authenticated.actor.session_id(), kind).await?;
        tx.commit().await?;
    }
    Ok(authenticated)
}

/// A user who may act: valid session, active account, no password change pending.
impl FromRequestParts<AppState> for Actor {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let authenticated = authenticate_request(parts, state).await?;
        if authenticated.must_change_password {
            return Err(IdentityError::PasswordChangeRequired.into());
        }
        Ok(authenticated.actor)
    }
}

/// Like `Actor`, but also accepts a user who still has to change a temporary password.
/// Only logout, `GET /me` and change password may use it.
impl FromRequestParts<AppState> for PendingActor {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(authenticate_request(parts, state).await?.actor))
    }
}

#[cfg(test)]
mod tests;
