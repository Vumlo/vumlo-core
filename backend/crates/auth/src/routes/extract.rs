//! Resolves a request to its session: finds the credential, enforces the CSRF rule for cookies,
//! resolves the session and refreshes it when it is due.
//!
//! Three channels carry a credential and each speaks for one audience (ADR-0095): a bearer token
//! and the `vumlo_session` cookie for the app, the `vumlo_console` cookie for the console. A token
//! that arrives through the channel of the other audience is not a credential.

use access::Audience;
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, header};
use axum_extra::extract::CookieJar;

use crate::AuthState;
use crate::model::SessionToken;
use crate::service::{self, AuthError, Authenticated};
use vumlo_config::ServerSettings;

pub const COOKIE_NAME: &str = "vumlo_session";
pub const CONSOLE_COOKIE_NAME: &str = "vumlo_console";

/// How a credential reached the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Channel {
    Bearer,
    SessionCookie,
    ConsoleCookie,
}

impl Channel {
    fn audience(self) -> Audience {
        match self {
            Self::Bearer | Self::SessionCookie => Audience::App,
            Self::ConsoleCookie => Audience::Console,
        }
    }
}

/// The credentials of a request, in the order they are tried. A bearer token stands alone. The
/// console cookie comes before the app cookie, unless the request names an `Origin` other than
/// the console's: on a host that serves both origins (`localhost` with two ports) the browser
/// sends both cookies everywhere, and the origin says who is speaking, so a request from the app
/// never rides the console cookie. A credential that is present but malformed is rejected, not
/// skipped; none at all is `Unauthenticated`.
fn candidates(
    headers: &HeaderMap,
    server: &ServerSettings,
) -> Result<Vec<(Channel, SessionToken)>, AuthError> {
    if let Some(value) = headers.get(header::AUTHORIZATION) {
        let text = value.to_str().map_err(|_| AuthError::Unauthenticated)?;
        let (scheme, token) = text.split_once(' ').ok_or(AuthError::Unauthenticated)?;
        if !scheme.eq_ignore_ascii_case("bearer") {
            return Err(AuthError::Unauthenticated);
        }
        let token = SessionToken::from_presented(token.trim()).ok_or(AuthError::Unauthenticated)?;
        return Ok(vec![(Channel::Bearer, token)]);
    }
    let jar = CookieJar::from_headers(headers);
    let from_console = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_none_or(|origin| origin == server.admin_origin);
    let mut found = Vec::new();
    for (name, channel) in [
        (CONSOLE_COOKIE_NAME, Channel::ConsoleCookie),
        (COOKIE_NAME, Channel::SessionCookie),
    ] {
        if channel == Channel::ConsoleCookie && !from_console {
            continue;
        }
        if let Some(cookie) = jar.get(name) {
            let token =
                SessionToken::from_presented(cookie.value()).ok_or(AuthError::Unauthenticated)?;
            found.push((channel, token));
        }
    }
    if found.is_empty() {
        return Err(AuthError::Unauthenticated);
    }
    Ok(found)
}

/// CSRF defence for cookie authentication: a request that is not read-only must come from the
/// origin of its audience (ADR-0041, ADR-0095): the public origin or a trusted frontend for the
/// app, exactly the admin origin for the console. Bearer requests are not sent by browsers on
/// their own, so they skip it.
pub(super) fn check_origin(
    method: &Method,
    headers: &HeaderMap,
    server: &ServerSettings,
    audience: Audience,
) -> Result<(), AuthError> {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return Ok(());
    }
    let allowed = |origin: &str| match audience {
        Audience::App => {
            origin == server.public_origin || server.allowed_origins.iter().any(|o| o == origin)
        }
        Audience::Console => origin == server.admin_origin,
    };
    match headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        Some(origin) if allowed(origin) => Ok(()),
        _ => Err(AuthError::OriginMismatch),
    }
}

/// Resolves the credential of a request to the session it names, and refreshes the session when
/// it is due. The caller turns the answer into an `Actor`.
pub async fn authenticate_request(
    parts: &Parts,
    state: &AuthState,
) -> Result<Authenticated, AuthError> {
    // A cookie that no longer names a session (a console session that ran out of time, say) must
    // not shadow the other cookie the browser sent along: try the next one.
    let mut authenticated = None;
    for (channel, token) in candidates(&parts.headers, &state.config.server)? {
        if channel != Channel::Bearer {
            check_origin(
                &parts.method,
                &parts.headers,
                &state.config.server,
                channel.audience(),
            )?;
        }
        let mut conn = state.db.acquire().await?;
        let found = service::authenticate(&mut conn, &token).await;
        drop(conn);
        match found {
            // A token is only good through the channel of its own audience.
            Ok(found) if found.audience == channel.audience() => {
                authenticated = Some(found);
                break;
            }
            Ok(_) | Err(AuthError::Unauthenticated) => {}
            Err(other) => return Err(other),
        }
    }
    let authenticated = authenticated.ok_or(AuthError::Unauthenticated)?;
    if let Some((kind, audience)) = authenticated.touch {
        let mut tx = state.db.begin().await?;
        service::touch_session(&mut tx, authenticated.session_id, kind, audience).await?;
        tx.commit().await?;
    }
    Ok(authenticated)
}

#[cfg(test)]
mod tests;
