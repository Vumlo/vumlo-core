//! Resolves a request to its session: finds the credential, enforces the CSRF rule for cookies,
//! resolves the session and refreshes it at most once an hour.

use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, header};
use axum_extra::extract::CookieJar;

use crate::auth::AuthState;
use crate::auth::model::SessionToken;
use crate::auth::service::{self, AuthError, Authenticated};
use vumlo_config::ServerSettings;

pub const COOKIE_NAME: &str = "vumlo_session";

enum Credential {
    Bearer(SessionToken),
    Cookie(SessionToken),
}

/// A bearer token wins over a cookie. A credential that is present but malformed is rejected,
/// not ignored.
fn credential(headers: &HeaderMap) -> Result<Credential, AuthError> {
    if let Some(value) = headers.get(header::AUTHORIZATION) {
        let text = value.to_str().map_err(|_| AuthError::Unauthenticated)?;
        let (scheme, token) = text.split_once(' ').ok_or(AuthError::Unauthenticated)?;
        if !scheme.eq_ignore_ascii_case("bearer") {
            return Err(AuthError::Unauthenticated);
        }
        return SessionToken::from_presented(token.trim())
            .map(Credential::Bearer)
            .ok_or(AuthError::Unauthenticated);
    }
    let jar = CookieJar::from_headers(headers);
    let cookie = jar.get(COOKIE_NAME).ok_or(AuthError::Unauthenticated)?;
    SessionToken::from_presented(cookie.value())
        .map(Credential::Cookie)
        .ok_or(AuthError::Unauthenticated)
}

/// CSRF defence for cookie authentication: a request that is not read-only must come from the
/// public origin or from one of the trusted frontends (ADR-0041). Bearer requests are not sent
/// by browsers on their own, so they skip it.
pub(super) fn check_origin(
    method: &Method,
    headers: &HeaderMap,
    server: &ServerSettings,
) -> Result<(), AuthError> {
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
        _ => Err(AuthError::OriginMismatch),
    }
}

/// Resolves the credential of a request to the session it names, and refreshes the session at most
/// once an hour. The caller turns the answer into an `Actor`.
pub async fn authenticate_request(
    parts: &Parts,
    state: &AuthState,
) -> Result<Authenticated, AuthError> {
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
        service::touch_session(&mut tx, authenticated.session_id, kind).await?;
        tx.commit().await?;
    }
    Ok(authenticated)
}

#[cfg(test)]
mod tests;
