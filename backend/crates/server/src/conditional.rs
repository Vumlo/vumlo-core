//! Conditional `GET` (ADR-0059): every JSON answer under `/api/v1` carries an `ETag` of its body, and a
//! request that names it in `If-None-Match` gets a `304` with no body. A client that keeps a copy (the
//! desktop app's offline store) asks "has this changed?" for the price of one round trip. The server does
//! the same work as before; what is saved is the transfer.

use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

use crate::error::ApiError;

/// The most a tagged answer may hold. Answers are built in memory already; one past this is a bug to see.
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

pub async fn layer(request: Request, next: Next) -> Response {
    if request.method() != Method::GET || !request.uri().path().starts_with("/api/v1/") {
        return next.run(request).await;
    }
    let presented: Vec<String> = request
        .headers()
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|tag| tag.trim().to_owned())
        .collect();
    let response = next.run(request).await;
    if !taggable(&response) {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_BODY_BYTES).await else {
        tracing::error!("conditional get: an answer is too large to tag");
        return ApiError::Internal.into_response();
    };
    let tag = etag(&bytes);
    let Ok(value) = HeaderValue::from_str(&tag) else {
        return ApiError::Internal.into_response();
    };
    // `no-cache` is "ask me every time", answered by the 304: a stale copy is never used unasked.
    // `private` keeps a shared cache from handing one person's answer to another.
    let cache = HeaderValue::from_static("private, no-cache");
    if matches(&presented, &tag) {
        let mut not_modified = Response::new(Body::empty());
        *not_modified.status_mut() = StatusCode::NOT_MODIFIED;
        not_modified.headers_mut().insert(header::ETAG, value);
        not_modified
            .headers_mut()
            .insert(header::CACHE_CONTROL, cache);
        return not_modified;
    }
    parts.headers.insert(header::ETAG, value);
    parts.headers.insert(header::CACHE_CONTROL, cache);
    Response::from_parts(parts, Body::from(bytes))
}

/// A successful JSON answer that did not already say how it may be cached. A credential never
/// comes from a `GET`, but an answer that sets a cookie or says `no-store` is left alone anyway.
fn taggable(response: &Response) -> bool {
    let headers = response.headers();
    response.status() == StatusCode::OK
        && !headers.contains_key(header::CACHE_CONTROL)
        && !headers.contains_key(header::SET_COOKIE)
        && !headers.contains_key(header::ETAG)
        && json(headers)
}

fn json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json"))
}

/// A weak tag (the same body, not a promise about bytes on the wire) of the first 128 bits of the hash.
fn etag(body: &[u8]) -> String {
    let hash = Sha256::digest(body);
    let hex: String = hash
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("W/\"{hex}\"")
}

/// `If-None-Match` compares weakly: `W/"a"` and `"a"` are the same tag, and `*` is any.
fn matches(presented: &[String], tag: &str) -> bool {
    let bare = |tag: &str| tag.strip_prefix("W/").unwrap_or(tag).to_owned();
    presented
        .iter()
        .any(|candidate| candidate == "*" || bare(candidate) == bare(tag))
}

#[cfg(test)]
mod tests;
