//! HTTP edge of `files`: the upload and the download (ADR-0069). The bytes are the raw body, so
//! these two routes sit outside the 2 MiB body limit, the 30 s timeout, the idempotency layer and
//! the conditional layer (`server::app`); an `ETag` and a `Range` are handled here.

use access::{Actor, ActorSource};
use api_types::{FileRefDto, ProblemDetails, UploadQuery};
use axum::Json;
use axum::body::Body;
use axum::extract::{FromRef, Path, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use sqlx::PgPool;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::model::Owner;
use super::service::{self, FileError};
use crate::LearningState;
use common::{ApiError, ValidQuery};

pub fn router<S>() -> OpenApiRouter<S>
where
    S: ActorSource + Clone + 'static,
    LearningState: FromRef<S>,
{
    OpenApiRouter::new()
        .routes(routes!(upload))
        .routes(routes!(download))
}

impl From<FileError> for ApiError {
    fn from(error: FileError) -> Self {
        match error {
            FileError::NotFound => Self::NotFound,
            FileError::Forbidden(policy) => policy.into(),
            FileError::Conflict(detail) => Self::Conflict(detail),
            FileError::Unusable { id } => {
                Self::invalid_field("file", format!("the file {id} cannot be used here"))
            }
            FileError::TooLarge => Self::Status(StatusCode::PAYLOAD_TOO_LARGE),
            FileError::Empty => Self::invalid_field("file", "the file is empty"),
            FileError::Content(e) => e.into(),
            FileError::Assessment(e) => e.into(),
            FileError::Files(files::FilesError::Db(e)) => Self::from_db("files", &e),
            FileError::Files(e) => {
                tracing::error!(error = %e, "files: storage error");
                Self::Internal
            }
            FileError::Db(e) => Self::from_db("files", &e),
        }
    }
}

const NOSNIFF: HeaderName = HeaderName::from_static("x-content-type-options");
const CSP: HeaderName = HeaderName::from_static("content-security-policy");

/// Stores the raw body as a file for an item or an assignment and returns its description. The
/// file stays unreferenced (and is swept after 24 hours) until a document or a submission names it.
///
/// The caller is authorized **before** the body is read: `for=item:{id}` needs the right to edit
/// the item (an archived course is `409`), `for=assignment:{id}` the right to submit work (an assignment that takes no submissions, whose window is closed or whose attempt limit is reached is `409`). The
/// content type is found from the bytes, not from the header, and only the six inline types of
/// ADR-0069 ever keep a type that a browser would show; the name is cleaned (no path, at most 200
/// characters). The body may be as large as `files.max_size` (`413` above it); an empty body is
/// `422`. This route is outside the 2 MiB body limit and the idempotency layer.
#[utoipa::path(
    post,
    path = "/files",
    tag = "files",
    security(("cookie" = []), ("bearer" = [])),
    params(UploadQuery),
    request_body(content = Vec<u8>, content_type = "application/octet-stream",
        description = "The raw bytes of the file"),
    responses(
        (status = 201, description = "The file is stored", body = FileRefDto),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the item or assignment but may not add files to it",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such item or assignment, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The course is archived, or (for an assignment) it takes no submissions, its submission window is closed or the attempt limit is reached", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 413, description = "The file is larger than `files.max_size`", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "`for` is not valid, or the body is empty", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn upload(
    State(state): State<LearningState>,
    actor: Actor,
    ValidQuery(query): ValidQuery<UploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<FileRefDto>), ApiError> {
    let owner = Owner::parse(&query.owner)
        .ok_or_else(|| ApiError::invalid_field("for", "must be item:<id> or assignment:<id>"))?;
    {
        // Released before the body is read: a slow upload holds no connection.
        let mut conn = state.db.acquire().await.map_err(FileError::from)?;
        service::authorize_upload(&mut conn, &actor, &owner, Utc::now()).await?;
    }
    let limit = usize::try_from(state.max_file_bytes).unwrap_or(usize::MAX);
    let data = axum::body::to_bytes(body, limit)
        .await
        .map_err(|error| read_failure(&error))?;
    let claimed = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let name = files::clean_name(&query.name);
    let (db, storage, max) = (&state.db, &state.files, state.max_file_bytes);
    let upload = Incoming {
        id: Uuid::now_v7(),
        data,
        name: &name,
        claimed,
    };
    let file = store_upload(db, storage, max, &actor, &owner, upload).await?;
    Ok((
        StatusCode::CREATED,
        Json(FileRefDto {
            id: file.id,
            name: file.name,
            mime: file.mime,
            size: file.size,
        }),
    ))
}

/// An upload whose bytes are in memory: the new file's id, the cleaned name, and the type the
/// client claimed (a hint only).
pub(super) struct Incoming<'a> {
    pub id: Uuid,
    pub data: axum::body::Bytes,
    pub name: &'a str,
    pub claimed: &'a str,
}

/// Writes the object, then its row, in a transaction of its own (the object first, so that
/// nothing slow runs inside the transaction). The sweep only walks rows, so an object whose row
/// never committed (the window closed while the body was on its way, the database failed) would
/// stay in storage for good: it is deleted here instead, best effort, and the original error is
/// the one returned.
pub(super) async fn store_upload(
    db: &PgPool,
    storage: &files::Storage,
    max_size: u64,
    actor: &Actor,
    owner: &Owner,
    upload: Incoming<'_>,
) -> Result<files::File, FileError> {
    let Incoming {
        id,
        data,
        name,
        claimed,
    } = upload;
    let stored = files::put(storage, id, data, max_size).await?;
    let mime = files::sniff(name, &stored.head, claimed);
    let registered = async {
        let mut tx = db.begin().await?;
        let file =
            service::register_upload(&mut tx, actor, owner, &stored, name, &mime, Utc::now())
                .await?;
        tx.commit().await?;
        Ok(file)
    }
    .await;
    if registered.is_err() {
        discard(db, storage, id).await;
    }
    registered
}

/// Removes the object of an upload that was not registered, and its row if there is one.
async fn discard(db: &PgPool, storage: &files::Storage, id: Uuid) {
    let removed = async {
        let mut conn = db.acquire().await?;
        files::delete(&mut conn, storage, id).await
    }
    .await;
    if let Err(error) = removed {
        tracing::warn!(file_id = %id, %error, "removing the object of a failed upload failed");
    }
}

/// Why the body could not be read: over the limit is `413`, anything else (a dropped connection)
/// is `422`.
fn read_failure(error: &axum::Error) -> ApiError {
    // The limit error comes from a crate that is not a dependency here, so it is found by its text.
    let mut cause: Option<&dyn std::error::Error> = Some(error);
    while let Some(current) = cause {
        if current.to_string().contains("length limit exceeded") {
            return ApiError::Status(StatusCode::PAYLOAD_TOO_LARGE);
        }
        cause = current.source();
    }
    ApiError::Unprocessable {
        detail: "the request body could not be read".to_owned(),
        errors: Vec::new(),
    }
}

/// The bytes of a file, streamed. Allowed if any item or submission that names the file lets the
/// caller read it, or, while nothing names it, to the uploader; every other case is `404`, never
/// `403` (ADR-0069).
///
/// Only `application/pdf`, `image/png`, `image/jpeg`, `image/gif`, `image/webp` and `text/plain`
/// are `inline`; everything else, SVG included, is an `attachment`. Every response has
/// `X-Content-Type-Options: nosniff` and, except for a PDF, `Content-Security-Policy: sandbox`
/// (a browser does not draw its PDF viewer in a sandboxed response; ADR-0069). The `ETag` is the
/// SHA-256 of the bytes and `If-None-Match` gives `304`. One `Range: bytes=a-b` (or `a-`, `-n`) is
/// served as `206`; a start past the end is `416`; any other form of the header is ignored.
#[utoipa::path(
    get,
    path = "/files/{id}",
    tag = "files",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "File id")),
    responses(
        (status = 200, description = "The bytes", content_type = "application/octet-stream",
            body = Vec<u8>),
        (status = 206, description = "The requested range of the bytes", content_type = "application/octet-stream",
            body = Vec<u8>),
        (status = 304, description = "`If-None-Match` names the current `ETag`"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such file, or not readable by the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 416, description = "The range starts past the end of the file", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn download(
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let file = {
        let mut conn = state.db.acquire().await.map_err(FileError::from)?;
        service::open_for_read(&mut conn, &actor, id).await?
    };
    let etag = format!("\"{}\"", file.sha256);
    let mut response = Response::new(Body::empty());
    let out = response.headers_mut();
    guard(out, &file.mime);
    out.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    out.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache"),
    );
    if let Ok(value) = HeaderValue::from_str(&etag) {
        out.insert(header::ETAG, value);
    }
    if matches_etag(&headers, &etag) {
        *response.status_mut() = StatusCode::NOT_MODIFIED;
        return Ok(response);
    }
    let size = u64::try_from(file.size).unwrap_or(0);
    let range = match headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_range(value, size))
    {
        Some(Err(())) => {
            let mut refused = ApiError::Status(StatusCode::RANGE_NOT_SATISFIABLE).into_response();
            guard(refused.headers_mut(), &file.mime);
            if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
                refused.headers_mut().insert(header::CONTENT_RANGE, value);
            }
            return Ok(refused);
        }
        Some(Ok(range)) => Some(range),
        None => None,
    };
    let result = files::read(
        &state.files,
        &file,
        range.map(|(first, last)| first..last + 1),
    )
    .await
    .map_err(FileError::from)?;
    let out = response.headers_mut();
    out.insert(header::CONTENT_TYPE, content_type(&file.mime));
    out.insert(
        header::CONTENT_DISPOSITION,
        disposition(&file.name, files::is_inline(&file.mime)),
    );
    let length = match range {
        Some((first, last)) => {
            *response.status_mut() = StatusCode::PARTIAL_CONTENT;
            if let Ok(value) = HeaderValue::from_str(&format!("bytes {first}-{last}/{size}")) {
                response.headers_mut().insert(header::CONTENT_RANGE, value);
            }
            last - first + 1
        }
        None => size,
    };
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    *response.body_mut() = Body::from_stream(result.into_stream());
    Ok(response)
}

/// What every answer about a file says: the type is what we said it is, and the document may do
/// nothing (no script, no forms, a unique origin) even if a browser shows it. The one exception is
/// the stored type `application/pdf`: Chrome and Edge do not draw their PDF viewer in a
/// CSP-sandboxed response, so a PDF gets `nosniff` but no `sandbox` (ADR-0069, 2026-11-01). The
/// decision is on the stored type, which is found from the bytes at upload, never on a name or a
/// header of the request.
fn guard(headers: &mut HeaderMap, mime: &str) {
    headers.insert(NOSNIFF, HeaderValue::from_static("nosniff"));
    if mime != "application/pdf" {
        headers.insert(CSP, HeaderValue::from_static("sandbox"));
    }
}

/// Does `If-None-Match` name this `ETag` (or `*`)? A weak validator matches too: for a `GET` the
/// comparison is weak (RFC 9110, 13.1.2).
fn matches_etag(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .map(|candidate| candidate.trim().trim_start_matches("W/"))
                .any(|candidate| candidate == "*" || candidate == etag)
        })
}

/// One byte range of a file of `size` bytes, as inclusive `(first, last)`. `None`: no usable
/// header (not `bytes=`, several ranges, or not a range at all), so the whole file is sent.
/// `Some(Err(()))`: well formed but outside the file.
fn parse_range(value: &str, size: u64) -> Option<Result<(u64, u64), ()>> {
    let spec = value.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (first, last) = spec.split_once('-')?;
    let (first, last) = (first.trim(), last.trim());
    if first.is_empty() {
        // `-n`: the last n bytes.
        let count: u64 = last.parse().ok()?;
        return Some(if count == 0 || size == 0 {
            Err(())
        } else {
            Ok((size.saturating_sub(count), size - 1))
        });
    }
    let first: u64 = first.parse().ok()?;
    let last: Option<u64> = if last.is_empty() {
        None
    } else {
        Some(last.parse().ok()?)
    };
    if last.is_some_and(|last| last < first) {
        return None;
    }
    if first >= size {
        return Some(Err(()));
    }
    Some(Ok((
        first,
        last.map_or(size - 1, |last| last.min(size - 1)),
    )))
}

/// The stored type, with the charset a plain text file is read in.
fn content_type(mime: &str) -> HeaderValue {
    let text = if mime == "text/plain" {
        "text/plain; charset=utf-8"
    } else {
        mime
    };
    HeaderValue::from_str(text)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"))
}

/// `inline` or `attachment`, with the name twice (RFC 6266): `filename*` carries it in UTF-8
/// (RFC 8187) and `filename` is an ASCII fallback in which nothing can end the quoted string or
/// start a parameter.
pub fn disposition(name: &str, inline: bool) -> HeaderValue {
    let kind = if inline { "inline" } else { "attachment" };
    let fallback: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && !matches!(c, '"' | '\\' | ';' | '%') || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::with_capacity(name.len());
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    HeaderValue::from_str(&format!(
        "{kind}; filename=\"{fallback}\"; filename*=UTF-8''{encoded}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
}

#[cfg(test)]
mod tests;
