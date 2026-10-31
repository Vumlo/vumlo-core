//! Uploaded files on the desktop (ADR-0069). The window never gets the file system, a shell or the opener
//! (ADR-0061): it asks for two commands, and the `vumlo-file` scheme serves the bytes to `<img>`, `<iframe>`
//! and `fetch`. The bytes and the token stay in Rust, as everything else of the network does (`core.rs`).
//!
//! The pure rules (names, the allowlist of what the system may open, the grammar of the arguments) are the
//! top of this file and are tested in `files/tests.rs`; the Tauri glue below them is thin.

use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::http::{self, HeaderName, HeaderValue, Method, StatusCode};
use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, Manager, Runtime, State, UriSchemeContext, UriSchemeResponder};

use crate::core::Core;

/// The extensions the system may be asked to open. A list of what is known to be harmless, not of what is
/// known to be dangerous: whatever is not here (every program, script and shortcut) is only shown in its folder.
const OPENABLE: [&str; 28] = [
    "pdf", "doc", "docx", "odt", "rtf", "txt", "md", "ppt", "pptx", "odp", "xls", "xlsx", "ods",
    "csv", "png", "jpg", "jpeg", "gif", "webp", "mp3", "wav", "mp4", "webm", "mov", "zip", "7z",
    "tar", "gz",
];

/// Whether the system may open a file of this name. Only the last extension counts (`report.pdf.exe` is an
/// `exe`), and a name that ends in a dot or a space does not qualify: Windows drops them, so `a.exe.` is `a.exe` there.
pub fn opens_directly(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ext)| OPENABLE.iter().any(|ok| ok.eq_ignore_ascii_case(ext)))
}

fn hidden_mark(c: char) -> bool {
    // The bidirectional controls, which make `evil\u{202e}fdp.exe` read as `evilexe.pdf`.
    c.is_control()
        || matches!(c, '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// A name that is safe to create in the Downloads folder: no folder part, no control or direction marks, none of
/// the characters Windows refuses, and never empty.
pub fn safe_name(raw: &str) -> String {
    let last = raw
        .rsplit(['/', '\\'])
        .find(|part| !part.trim().is_empty())
        .unwrap_or("");
    let cleaned: String = last
        .chars()
        .filter(|c| !hidden_mark(*c))
        .map(|c| if ":*?\"<>|".contains(c) { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        "file".to_owned()
    } else {
        cleaned.to_owned()
    }
}

/// `dir/name`, or `dir/stem (1).ext`, `dir/stem (2).ext` ... for the first that does not exist.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let taken = |path: &Path| path.symlink_metadata().is_ok();
    let first = dir.join(name);
    if !taken(&first) {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|path| !taken(path))
        .unwrap_or(first)
}

/// Writes `bytes` to a new file in `dir` named by `unique_path`. The file is created exclusively, so a name
/// taken between the check and the write moves on to the next counter instead of replacing someone's file.
pub fn save_new(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    for _ in 0..100 {
        let path = unique_path(dir, name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(bytes)?;
                return Ok(path);
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(ErrorKind::AlreadyExists.into())
}

/// A UUID as the server writes ids: 36 characters, hex with dashes in the places of the groups.
fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text.bytes().enumerate().all(|(at, b)| match at {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

/// The grammar of the server's `for` (`Owner::parse`): `item:<uuid>` or `assignment:<uuid>`.
pub fn is_upload_target(text: &str) -> bool {
    text.split_once(':')
        .is_some_and(|(kind, id)| matches!(kind, "item" | "assignment") && is_uuid(id))
}

/// The file an address of the `vumlo-file` scheme names: its path is exactly `/<uuid>`, whatever the host.
pub fn vumlo_file_id(path: &str) -> Option<&str> {
    path.strip_prefix('/').filter(|id| is_uuid(id))
}

fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = text.get(at + 1..at + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `POST /files?for=..&name=..` for the target and the percent-encoded name the window sent. An argument the
/// server would refuse is refused here with its status, `"422: why"`, the form every error of the commands has.
pub fn upload_path(target: &str, encoded_name: &str) -> Result<String, String> {
    if !is_upload_target(target) {
        return Err("422: the target must be item:<id> or assignment:<id>".to_owned());
    }
    let name = percent_decode(encoded_name).ok_or("422: the name is not valid text")?;
    if name.is_empty() || name.chars().count() > 2000 {
        return Err("422: the name must be 1 to 2000 characters".to_owned());
    }
    Ok(format!(
        "/files?for={}&name={}",
        percent_encode(target),
        percent_encode(&name)
    ))
}

/// The file name of a `Content-Disposition`: `filename*` (RFC 8187, UTF-8) first, then `filename`.
pub fn disposition_name(header: &str) -> Option<String> {
    let param = |name: &str| {
        header.split(';').skip(1).find_map(|part| {
            let (key, value) = part.split_once('=')?;
            key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
        })
    };
    if let Some(value) = param("filename*") {
        let (charset, rest) = value.split_once('\'')?;
        let encoded = rest.split_once('\'')?.1;
        if charset.eq_ignore_ascii_case("utf-8") {
            return percent_decode(encoded);
        }
    }
    param("filename").map(|value| value.trim_matches('"').to_owned())
}

/// `"<status>: <detail>"` for an answer that is not a success. The window reads the status from it.
pub fn error_text(status: u16, body: &[u8]) -> String {
    let problem: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let detail = ["detail", "title"]
        .iter()
        .find_map(|key| problem.get(key)?.as_str())
        .unwrap_or("");
    format!("{status}: {detail}")
}

/// Whether a page at `origin` is the app's own window (the page of the app is `tauri://localhost`, or
/// `http(s)://tauri.localhost` on Windows and Android; in a debug build, the dev server).
pub fn own_origin(origin: &str) -> bool {
    matches!(
        origin,
        "tauri://localhost" | "http://tauri.localhost" | "https://tauri.localhost"
    ) || (cfg!(debug_assertions) && origin == "http://localhost:3000")
}

/// What the server tells about a stored file (`FileRefDto`).
#[derive(Serialize, Deserialize)]
pub struct FileRef {
    id: String,
    name: String,
    mime: String,
    size: i64,
}

/// What `open_file` did, for the window to say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenOutcome {
    /// The system's program for this kind of file was started.
    Opened,
    /// The file is in Downloads and its folder was shown (the kind of file is not one to open directly).
    Revealed,
    /// The file is in Downloads; the system could not show it.
    SavedOnly,
}

// --- the commands -------------------------------------------------------------------------------------

/// Uploads a file. The bytes are the raw body of the call (Tauri's binary IPC) and the arguments are headers:
/// `x-vumlo-target` (`item:<id>` or `assignment:<id>`), `x-vumlo-name` (percent-encoded) and `content-type`.
/// Answers with the stored `FileRef`, or with `"<status>: <detail>"`.
#[tauri::command]
pub async fn upload_file(
    core: State<'_, Arc<Core>>,
    request: Request<'_>,
) -> Result<FileRef, String> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("422: the file must be sent as bytes".to_owned());
    };
    let header = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok());
    let path = upload_path(
        header("x-vumlo-target").unwrap_or_default(),
        header("x-vumlo-name").unwrap_or_default(),
    )?;
    let mime = header("content-type").unwrap_or("application/octet-stream");
    // ponytail: the bytes are copied once more to hand them to the HTTP client; stream them when an ADR adds a stream crate
    let reply = core
        .request_raw(
            "POST",
            &path,
            &[("content-type", mime)],
            Some(bytes.clone()),
        )
        .await?;
    if !(200..300).contains(&reply.status) {
        return Err(error_text(reply.status, &reply.body));
    }
    serde_json::from_slice(&reply.body).map_err(|e| e.to_string())
}

/// Saves a file to the system's Downloads folder and shows it: it is opened in the program of the system when
/// its kind is on the allowlist, otherwise its folder is shown. Nothing is ever run.
#[tauri::command]
pub async fn open_file(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    id: String,
) -> Result<OpenOutcome, String> {
    if !is_uuid(&id) {
        return Err("422: not a file id".to_owned());
    }
    let reply = core
        .request_raw("GET", &format!("/files/{id}"), &[], None)
        .await?;
    if reply.status != 200 {
        return Err(error_text(reply.status, &reply.body));
    }
    let name = safe_name(
        &reply
            .header("content-disposition")
            .and_then(disposition_name)
            .unwrap_or_else(|| id.clone()),
    );
    let dir = app.path().download_dir().map_err(|e| e.to_string())?;
    let (saved_name, bytes) = (name.clone(), reply.body);
    let path = tauri::async_runtime::spawn_blocking(move || save_new(&dir, &saved_name, &bytes))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let shown = if opens_directly(&name) {
        tauri_plugin_opener::open_path(&path, None::<&str>).map(|()| OpenOutcome::Opened)
    } else {
        tauri_plugin_opener::reveal_item_in_dir(&path).map(|()| OpenOutcome::Revealed)
    };
    // The file stays in Downloads whatever the system does with it.
    Ok(shown.unwrap_or(OpenOutcome::SavedOnly))
}

// --- the `vumlo-file` scheme ----------------------------------------------------------------------------

const EXPOSED: &str = "Content-Range, Content-Length, Content-Type";

fn answer(status: u16, headers: &[(&str, &str)], body: Vec<u8>) -> http::Response<Vec<u8>> {
    let mut response = http::Response::new(body);
    *response.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    for (name, value) in headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            response.headers_mut().append(name, value);
        }
    }
    response
}

/// `vumlo-file://localhost/<uuid>` (`http://vumlo-file.localhost/<uuid>` on Windows and Android) is the file with
/// that id, fetched with the token, which the page does not have. A `Range` is forwarded, the server's answer
/// (200, 206, 416, 404) is passed on, and a page of the app may read it with `fetch`.
pub fn serve<R: Runtime>(
    context: UriSchemeContext<'_, R>,
    request: http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let core = context.app_handle().try_state::<Arc<Core>>();
    let core = core.map(|state| state.inner().clone());
    tauri::async_runtime::spawn(async move {
        responder.respond(serve_one(core, &request).await);
    });
}

async fn serve_one(
    core: Option<Arc<Core>>,
    request: &http::Request<Vec<u8>>,
) -> http::Response<Vec<u8>> {
    let origin = request
        .headers()
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .filter(|origin| own_origin(origin));
    let mut cors = vec![("vary", "Origin"), ("x-content-type-options", "nosniff")];
    if let Some(origin) = origin {
        cors.push(("access-control-allow-origin", origin));
        cors.push(("access-control-expose-headers", EXPOSED));
    }
    if request.method() == Method::OPTIONS {
        cors.push(("access-control-allow-methods", "GET"));
        cors.push(("access-control-allow-headers", "Range"));
        return answer(204, &cors, Vec::new());
    }
    if request.method() != Method::GET {
        return answer(405, &cors, Vec::new());
    }
    let Some(id) = vumlo_file_id(request.uri().path()) else {
        return answer(404, &cors, Vec::new());
    };
    let Some(core) = core else {
        return answer(503, &cors, Vec::new());
    };
    let range = request
        .headers()
        .get("range")
        .and_then(|v| v.to_str().ok())
        .map(|range| ("range", range));
    let reply = core
        .request_raw("GET", &format!("/files/{id}"), range.as_slice(), None)
        .await;
    let Ok(reply) = reply else {
        return answer(502, &cors, Vec::new());
    };
    let mut headers = cors;
    for name in [
        "content-type",
        "content-range",
        "content-disposition",
        "accept-ranges",
    ] {
        if let Some(value) = reply.header(name) {
            headers.push((name, value));
        }
    }
    let length = reply.body.len().to_string();
    headers.push(("content-length", &length));
    let mut response = answer(reply.status, &headers, Vec::new());
    *response.body_mut() = reply.body;
    response
}

#[cfg(test)]
mod tests;
