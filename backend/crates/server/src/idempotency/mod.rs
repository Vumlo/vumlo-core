//! The `Idempotency-Key` header on `POST` (ADR-0058). A client that did not see the answer to a `POST`
//! sends it again with the same key and the same body and gets the answer of the first time, not a second
//! effect. The header is optional: a request without it is untouched.
//!
//! This is a layer, not a route: it sits innermost around the whole API, so the handlers do not know it
//! exists. The first request runs on its own task, so a client that goes away does not leave a record
//! that never finishes.

mod repo;

use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::app::{AppState, MAX_BODY_BYTES};
use crate::error::ApiError;
use crate::identity::COOKIE_NAME;
use repo::{Claim, Stored};

pub const KEY_HEADER: HeaderName = HeaderName::from_static("idempotency-key");
/// Added to an answer that is a replay, so a client (and a test) can tell.
pub const REPLAYED_HEADER: HeaderName = HeaderName::from_static("idempotency-replayed");

const MAX_KEY_LEN: usize = 255;
/// What is stored of an answer. The answers of a `POST` are small; one that is not is a bug to see, not to keep.
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
/// What a client is told to wait when the first request with its key is still running.
const RETRY_AFTER_SECS: &str = "1";

type Digest32 = [u8; 32];

pub async fn layer(State(state): State<AppState>, request: Request, next: Next) -> Response {
    match run(state, request, next).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn run(state: AppState, request: Request, next: Next) -> Result<Response, ApiError> {
    if !applies(request.method(), request.uri().path()) {
        return Ok(next.run(request).await);
    }
    let Some(key) = key_of(request.headers())? else {
        return Ok(next.run(request).await);
    };
    // No credential: the handler will refuse the request, and there is nobody to scope a key to.
    let Some(scope) = scope_of(request.headers()) else {
        return Ok(next.run(request).await);
    };

    let (parts, body) = request.into_parts();
    let body = to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|_| ApiError::Status(StatusCode::PAYLOAD_TOO_LARGE))?;
    let request_hash = digest(&[
        parts
            .uri
            .path_and_query()
            .map_or("", |p| p.as_str())
            .as_bytes(),
        &[0],
        &body,
    ]);

    let mut conn = state
        .db
        .acquire()
        .await
        .map_err(|e| ApiError::from_db("idempotency", &e))?;
    let claim = repo::claim(&mut conn, &scope, &key, &request_hash)
        .await
        .map_err(|e| ApiError::from_db("idempotency", &e))?;
    drop(conn);
    match claim {
        Claim::Replay(stored) => return Ok(replay(stored)),
        Claim::Mismatch => {
            return Err(ApiError::invalid_field(
                "Idempotency-Key",
                "this key was used for a different request",
            ));
        }
        Claim::Running => {
            let mut response =
                ApiError::Conflict("a request with this Idempotency-Key is still running")
                    .into_response();
            response.headers_mut().insert(
                header::RETRY_AFTER,
                HeaderValue::from_static(RETRY_AFTER_SECS),
            );
            return Ok(response);
        }
        Claim::Fresh => {}
    }

    let request = Request::from_parts(parts, Body::from(body));
    let db = state.db.clone();
    let task_key = key.clone();
    let task =
        tokio::spawn(async move { finish(&db, &scope, &task_key, next.run(request).await).await });
    match task.await {
        Ok(response) => Ok(response),
        Err(_) => {
            // The handler panicked: nothing was answered, so the key is free for the retry.
            forget(&state.db, &scope, &key).await;
            Err(ApiError::Internal)
        }
    }
}

/// Only a `POST` to the API proper. Sign-in and the other `/auth` calls answer with credentials, which
/// must never be kept (AGENTS §11); a `PUT` and a `DELETE` repeat harmlessly by themselves.
fn applies(method: &Method, path: &str) -> bool {
    method == Method::POST && path.starts_with("/api/v1/") && !path.starts_with("/api/v1/auth/")
}

/// The key, `None` without the header, an error for one that cannot be a key.
fn key_of(headers: &HeaderMap) -> Result<Option<String>, ApiError> {
    let mut values = headers.get_all(KEY_HEADER).iter();
    let Some(first) = values.next() else {
        return Ok(None);
    };
    let bad = || {
        ApiError::invalid_field(
            "Idempotency-Key",
            "must be 1 to 255 printable characters, once",
        )
    };
    if values.next().is_some() {
        return Err(bad());
    }
    let key = first.to_str().map_err(|_| bad())?;
    if key.is_empty() || key.len() > MAX_KEY_LEN || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(bad());
    }
    Ok(Some(key.to_owned()))
}

/// Who the key belongs to: a hash of the credential that came with the request, whatever its kind. The
/// handler decides whether the credential is good; a bad one gets a `401`, which is not kept.
fn scope_of(headers: &HeaderMap) -> Option<Digest32> {
    if let Some(value) = headers.get(header::AUTHORIZATION) {
        return Some(digest(&[b"authorization\0", value.as_bytes()]));
    }
    let jar = CookieJar::from_headers(headers);
    jar.get(COOKIE_NAME)
        .map(|cookie| digest(&[b"cookie\0", cookie.value().as_bytes()]))
}

fn digest(parts: &[&[u8]]) -> Digest32 {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

/// An answer worth giving again. A server error, a timeout, a throttle and a refused credential say
/// nothing about the request itself, so the retry runs it.
fn storable(status: StatusCode) -> bool {
    status.is_success() || (status.is_client_error() && !matches!(status.as_u16(), 401 | 408 | 429))
}

async fn finish(db: &PgPool, scope: &Digest32, key: &str, response: Response) -> Response {
    let (parts, body) = response.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_RESPONSE_BYTES).await else {
        tracing::error!("idempotency: an answer is too large to keep");
        forget(db, scope, key).await;
        return ApiError::Internal.into_response();
    };
    if storable(parts.status) {
        let stored = Stored {
            status: parts.status.as_u16() as i16,
            content_type: parts
                .headers
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned),
            body: bytes.to_vec(),
        };
        let kept = match db.acquire().await {
            Ok(mut conn) => repo::complete(&mut conn, scope, key, &stored).await,
            Err(e) => Err(e),
        };
        if let Err(e) = kept {
            // The answer goes out anyway; the record is freed so the retry is not told "still running".
            tracing::warn!(error = %e, "idempotency: the answer could not be kept");
            forget(db, scope, key).await;
        }
    } else {
        forget(db, scope, key).await;
    }
    Response::from_parts(parts, Body::from(bytes))
}

async fn forget(db: &PgPool, scope: &Digest32, key: &str) {
    let freed = match db.acquire().await {
        Ok(mut conn) => repo::release(&mut conn, scope, key).await,
        Err(e) => Err(e),
    };
    if let Err(e) = freed {
        tracing::warn!(error = %e, "idempotency: a key could not be freed");
    }
}

fn replay(stored: Stored) -> Response {
    let mut response = Response::new(Body::from(stored.body));
    *response.status_mut() = StatusCode::from_u16(stored.status as u16).unwrap_or(StatusCode::OK);
    if let Some(content_type) = stored
        .content_type
        .and_then(|v| HeaderValue::from_str(&v).ok())
    {
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, content_type);
    }
    response
        .headers_mut()
        .insert(REPLAYED_HEADER, HeaderValue::from_static("true"));
    response
}

#[cfg(test)]
mod tests;
