//! Uploading and downloading files over HTTP (ADR-0069): who may upload, what is stored, and what
//! the download says about itself. Owners (items, submissions) do not name files yet, so the
//! reference rows are inserted with SQL where a test needs a file to belong to one.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ::common::Role;
use common::{Account, Env, Resp, db_test, id_of};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

const PDF: &[u8] = b"%PDF-1.4\n1 0 obj\n<<>>\nendobj\n";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0";

struct World {
    env: Env,
    owner: Account,
    student: Account,
    stranger: Account,
    course: Uuid,
    /// A published text item: students can see it, only staff can edit it.
    item: Uuid,
    /// A text item nobody has published.
    draft_item: Uuid,
    assignment: Uuid,
    dir: PathBuf,
}

async fn world(pool: PgPool) -> World {
    world_with(pool, None).await
}

async fn world_with(pool: PgPool, max_size: Option<u64>) -> World {
    let dir = std::env::temp_dir().join(format!("vumlo-files-{}", Uuid::now_v7()));
    let mut settings = common::test_settings();
    settings.files.path = Some(dir.clone());
    if let Some(max_size) = max_size {
        settings.files.max_size = max_size;
    }
    let env = Env::with_settings(pool, settings).await;
    let owner = env.user("owner@x.edu", Role::Teacher).await;
    let student = env.user("student@x.edu", Role::Student).await;
    let stranger = env.user("stranger@x.edu", Role::Teacher).await;
    let (course, module) = env.course(&owner, &[&student]).await;
    let item = text_item(&env, &owner, module).await;
    env.ok(
        "POST",
        &owner.token,
        &format!("/items/{item}/publish"),
        None,
        200,
    )
    .await;
    let draft_item = text_item(&env, &owner, module).await;
    let assignment = env.assignment(&owner, module, "10").await;
    World {
        env,
        owner,
        student,
        stranger,
        course,
        item,
        draft_item,
        assignment,
        dir,
    }
}

async fn text_item(env: &Env, owner: &Account, module: Uuid) -> Uuid {
    id_of(
        &env.ok(
            "POST",
            &owner.token,
            &format!("/modules/{module}/items"),
            Some(json!({"kind": "text", "title": "L"})),
            201,
        )
        .await,
    )
}

/// Percent-encodes everything but unreserved characters, for a query value.
fn pct(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

async fn upload_as(
    w: &World,
    token: &str,
    owner: &str,
    name: &str,
    claimed: &str,
    bytes: &[u8],
) -> Resp {
    w.env
        .http
        .post(&format!("/files?for={owner}&name={}", pct(name)))
        .bearer(token)
        .header("content-type", claimed)
        .body(bytes.to_vec())
        .send()
        .await
}

/// The owner uploads for the published item.
async fn upload(w: &World, name: &str, claimed: &str, bytes: &[u8]) -> Resp {
    upload_as(
        w,
        &w.owner.token,
        &format!("item:{}", w.item),
        name,
        claimed,
        bytes,
    )
    .await
}

async fn stored(w: &World, name: &str, claimed: &str, bytes: &[u8]) -> Uuid {
    let response = upload(w, name, claimed, bytes).await;
    assert_eq!(
        response.status,
        201,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    id_of(&response.json())
}

fn get(w: &World, token: &str, id: Uuid) -> common::Req {
    w.env.http.get(&format!("/files/{id}")).bearer(token)
}

async fn download(w: &World, token: &str, id: Uuid) -> Resp {
    get(w, token, id).send().await
}

async fn rows(pool: &PgPool) -> i64 {
    sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM files.files"#)
        .fetch_one(pool)
        .await
        .expect("count files")
}

fn objects(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .map(|entry| {
            let path = entry.expect("entry").path();
            if path.is_dir() { objects(&path) } else { 1 }
        })
        .sum()
}

async fn name_in_item(pool: &PgPool, item: Uuid, file: Uuid, published: bool) {
    sqlx::query!(
        "INSERT INTO content.item_files (item_id, file_id, published) VALUES ($1, $2, $3)",
        item,
        file,
        published
    )
    .execute(pool)
    .await
    .expect("reference an item file");
}

/// Sends `request` on a fresh connection and returns the status and the body once the head and
/// `Content-Length` bytes are in. The caller decides how much of its own body it sends.
async fn raw(base: &str, request: String) -> (u16, Vec<u8>) {
    let address = base.trim_start_matches("http://").to_owned();
    tokio::task::spawn_blocking(move || {
        let mut stream = TcpStream::connect(address).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        stream.write_all(request.as_bytes()).expect("send");
        let mut received = Vec::new();
        let mut chunk = [0_u8; 4096];
        let (head_end, length) = loop {
            let read = stream
                .read(&mut chunk)
                .expect("the server answered in time");
            assert!(read > 0, "the server closed without an answer");
            received.extend_from_slice(&chunk[..read]);
            if let Some(end) = received.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&received[..end]).to_ascii_lowercase();
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                break (end + 4, length);
            }
        };
        while received.len() < head_end + length {
            let read = stream.read(&mut chunk).expect("the body arrived");
            assert!(read > 0, "the body was cut short");
            received.extend_from_slice(&chunk[..read]);
        }
        let head = String::from_utf8_lossy(&received[..head_end]).into_owned();
        let status = head
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .expect("a status line");
        (status, received[head_end..head_end + length].to_vec())
    })
    .await
    .expect("the raw request ran")
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// =============================================================================================
// Upload
// =============================================================================================

#[tokio::test]
async fn upload_then_download_returns_identical_bytes() {
    db_test(async |pool| {
        let w = world(pool).await;
        let bytes: Vec<u8> = (0..5000_u32).map(|i| (i * 7 % 251) as u8).collect();
        let response = upload(&w, "data.bin", "application/octet-stream", &bytes).await;
        assert_eq!(response.status, 201);
        let body = response.json();
        assert_eq!(body["name"], "data.bin");
        assert_eq!(body["mime"], "application/octet-stream");
        assert_eq!(body["size"], 5000);
        let id = id_of(&body);

        // Nothing names the file yet, so only the uploader reads it.
        let back = download(&w, &w.owner.token, id).await;
        assert_eq!(back.status, 200);
        assert_eq!(back.body, bytes);
        assert_eq!(back.header("etag"), format!("\"{}\"", sha256(&bytes)));
        assert_eq!(back.header("content-length"), "5000");
        assert_eq!(back.header("accept-ranges"), "bytes");
        assert_eq!(back.header("cache-control"), "private, no-cache");
        assert_eq!(download(&w, &w.stranger.token, id).await.status, 404);
    })
    .await;
}

#[tokio::test]
async fn upload_is_authorized_before_the_body_is_read() {
    db_test(async |pool| {
        let w = world(pool).await;
        // The head promises a body that is never sent: the answer can only come from a server
        // that decided without it.
        let head = |token: &str, owner: &str| {
            format!(
                "POST /api/v1/files?for={owner}&name=a.txt HTTP/1.1\r\nhost: x\r\n\
                 authorization: Bearer {token}\r\ncontent-length: 1000\r\n\
                 content-type: text/plain\r\n\r\n"
            )
        };
        let item = format!("item:{}", w.item);
        let (status, body) = raw(&w.env.base, head(&w.student.token, &item)).await;
        assert_eq!(status, 403, "{}", String::from_utf8_lossy(&body));
        let (status, _) = raw(&w.env.base, head(&w.stranger.token, &item)).await;
        assert_eq!(status, 404);
        let draft = format!("item:{}", w.draft_item);
        let (status, _) = raw(&w.env.base, head(&w.student.token, &draft)).await;
        assert_eq!(status, 404);
        let (status, _) = raw(&w.env.base, head("not-a-token", &item)).await;
        assert_eq!(status, 401);
        // The same through a client: refused, and nothing stored.
        let refused = upload_as(&w, &w.student.token, &item, "a.txt", "text/plain", b"hi").await;
        assert_eq!(refused.problem(403).status, 403);
        assert_eq!(rows(&w.env.pool).await, 0);
        assert_eq!(objects(&w.dir), 0);
    })
    .await;
}

#[tokio::test]
async fn a_student_uploads_to_an_assignment_and_not_to_an_item() {
    db_test(async |pool| {
        let w = world(pool).await;
        let own = upload_as(
            &w,
            &w.student.token,
            &format!("assignment:{}", w.assignment),
            "essay.txt",
            "text/plain",
            b"my essay",
        )
        .await;
        assert_eq!(own.status, 201);
        let other = upload_as(
            &w,
            &w.stranger.token,
            &format!("assignment:{}", w.assignment),
            "essay.txt",
            "text/plain",
            b"x",
        )
        .await;
        assert_eq!(other.status, 404);
        let bad = |owner: &str| {
            let owner = owner.to_owned();
            let w = &w;
            async move { upload_as(w, &w.owner.token, &owner, "a.txt", "text/plain", b"x").await }
        };
        for owner in [
            "",
            "item",
            "item:nope",
            "course:1",
            &format!("module:{}", w.item),
        ] {
            let response = bad(owner).await;
            let problem = response.problem(422);
            assert!(
                problem.errors.iter().any(|e| e.field == "for"),
                "{owner}: {problem:?}"
            );
        }
        assert_eq!(
            bad(&format!("item:{}", Uuid::now_v7())).await.status,
            404,
            "an item that does not exist"
        );
    })
    .await;
}

#[tokio::test]
async fn upload_zero_bytes_is_422_and_stores_nothing() {
    db_test(async |pool| {
        let w = world(pool).await;
        let response = upload(&w, "empty.txt", "text/plain", b"").await;
        let problem = response.problem(422);
        assert_eq!(problem.errors.len(), 1);
        assert_eq!(problem.errors[0].field, "file");
        assert_eq!(rows(&w.env.pool).await, 0);
        assert_eq!(objects(&w.dir), 0);
    })
    .await;
}

#[tokio::test]
async fn upload_over_max_size_is_413_and_stores_nothing() {
    db_test(async |pool| {
        let w = world_with(pool, Some(1000)).await;
        let fits = upload(&w, "fits.bin", "application/octet-stream", &[7; 1000]).await;
        assert_eq!(fits.status, 201);
        assert_eq!(objects(&w.dir), 1);

        let over = upload(&w, "big.bin", "application/octet-stream", &[7; 1001]).await;
        over.problem(413);

        // No Content-Length to refuse early: the limit is met while the body is read.
        let item = format!("item:{}", w.item);
        let chunk = "x".repeat(1001);
        let request = format!(
            "POST /api/v1/files?for={item}&name=big.bin HTTP/1.1\r\nhost: x\r\n\
             authorization: Bearer {}\r\ntransfer-encoding: chunked\r\n\r\n3e9\r\n{chunk}\r\n0\r\n\r\n",
            w.owner.token
        );
        let (status, body) = raw(&w.env.base, request).await;
        assert_eq!(status, 413, "{}", String::from_utf8_lossy(&body));
        let problem: Value = serde_json::from_slice(&body).expect("Problem Details");
        assert_eq!(problem["status"], 413);

        assert_eq!(rows(&w.env.pool).await, 1, "only the file that fit");
        assert_eq!(objects(&w.dir), 1);
    })
    .await;
}

#[tokio::test]
async fn upload_to_an_archived_course_is_409() {
    db_test(async |pool| {
        let w = world(pool).await;
        w.env
            .ok(
                "POST",
                &w.owner.token,
                &format!("/courses/{}/archive", w.course),
                None,
                200,
            )
            .await;
        let for_item = upload(&w, "a.txt", "text/plain", b"x").await;
        for_item.problem(409);
        let for_assignment = upload_as(
            &w,
            &w.student.token,
            &format!("assignment:{}", w.assignment),
            "a.txt",
            "text/plain",
            b"x",
        )
        .await;
        for_assignment.problem(409);
        assert_eq!(rows(&w.env.pool).await, 0);
        assert_eq!(objects(&w.dir), 0);
    })
    .await;
}

#[tokio::test]
async fn upload_larger_than_2_mib_is_accepted() {
    db_test(async |pool| {
        let w = world(pool).await;
        let bytes: Vec<u8> = (0..3 * 1024 * 1024_u32).map(|i| (i % 253) as u8).collect();
        let id = stored(&w, "big.bin", "application/octet-stream", &bytes).await;
        let back = download(&w, &w.owner.token, id).await;
        assert_eq!(back.status, 200);
        assert!(back.body == bytes, "the 3 MiB come back unchanged");
        // The 2 MiB limit still holds for every other route.
        let big = json!({"title": "x".repeat(3 * 1024 * 1024)});
        let response = w
            .env
            .http
            .post(&format!("/courses/{}/modules", w.course))
            .bearer(&w.owner.token)
            .json(&big)
            .send()
            .await;
        response.problem(413);
    })
    .await;
}

#[tokio::test]
async fn upload_skips_the_idempotency_layer() {
    db_test(async |pool| {
        let w = world(pool).await;
        let mut ids = Vec::new();
        for _ in 0..2 {
            let response = w
                .env
                .http
                .post(&format!("/files?for=item:{}&name=a.txt", w.item))
                .bearer(&w.owner.token)
                .header("idempotency-key", "same-key-1")
                .body(b"same bytes".to_vec())
                .send()
                .await;
            assert_eq!(response.status, 201);
            assert!(response.headers.get("idempotency-replayed").is_none());
            ids.push(id_of(&response.json()));
        }
        assert_ne!(ids[0], ids[1], "the same bytes twice are two files");
        assert_eq!(rows(&w.env.pool).await, 2);
    })
    .await;
}

// =============================================================================================
// Download
// =============================================================================================

#[tokio::test]
async fn download_headers_for_pdf_png_text() {
    db_test(async |pool| {
        let w = world(pool).await;
        let cases: [(&str, &[u8], &str, &str); 3] = [
            ("report.pdf", PDF, "application/pdf", "application/pdf"),
            ("pixel.png", PNG, "image/png", "image/png"),
            (
                "notes.txt",
                b"plain words",
                "text/plain",
                "text/plain; charset=utf-8",
            ),
        ];
        for (name, bytes, mime, sent) in cases {
            let id = stored(&w, name, "application/octet-stream", bytes).await;
            let response = download(&w, &w.owner.token, id).await;
            assert_eq!(response.status, 200, "{name}");
            assert_eq!(response.header("content-type"), sent, "{name}");
            let disposition = response.header("content-disposition");
            assert!(disposition.starts_with("inline;"), "{name}: {disposition}");
            let stored_mime = w.env.pool.clone();
            let row = sqlx::query_scalar!("SELECT mime FROM files.files WHERE id = $1", id)
                .fetch_one(&stored_mime)
                .await
                .expect("row");
            assert_eq!(row, mime);
        }
    })
    .await;
}

#[tokio::test]
async fn download_headers_for_svg_docx_and_fake_pdf() {
    db_test(async |pool| {
        let w = world(pool).await;
        let docx = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
        let cases: [(&str, &[u8], &str, &str); 3] = [
            (
                "logo.svg",
                b"<svg xmlns='x'><script/></svg>",
                "image/svg+xml",
                "image/svg+xml",
            ),
            ("essay.docx", b"PK\x03\x04rest", docx, docx),
            // A claim and a name are not evidence: these bytes are not a PDF.
            (
                "fake.pdf",
                b"<html><script>1</script></html>",
                "application/pdf",
                "application/octet-stream",
            ),
        ];
        for (name, bytes, claimed, expected) in cases {
            let upload = upload(&w, name, claimed, bytes).await;
            assert_eq!(upload.status, 201, "{name}");
            assert_eq!(upload.json()["mime"], expected, "{name}");
            let response = download(&w, &w.owner.token, id_of(&upload.json())).await;
            assert_eq!(response.header("content-type"), expected, "{name}");
            let disposition = response.header("content-disposition");
            assert!(
                disposition.starts_with("attachment;"),
                "{name}: {disposition}"
            );
        }
    })
    .await;
}

/// The four answers a file can give: the bytes, a range, "not modified" and "range not satisfiable".
async fn all_kinds_of_answer(w: &World, id: Uuid, bytes: &[u8]) -> [(u16, Resp); 4] {
    let etag = format!("\"{}\"", sha256(bytes));
    [
        (200, download(w, &w.owner.token, id).await),
        (
            206,
            get(w, &w.owner.token, id)
                .header("range", "bytes=0-3")
                .send()
                .await,
        ),
        (
            304,
            get(w, &w.owner.token, id)
                .header("if-none-match", &etag)
                .send()
                .await,
        ),
        (
            416,
            get(w, &w.owner.token, id)
                .header("range", &format!("bytes={}-", bytes.len()))
                .send()
                .await,
        ),
    ]
}

/// ADR-0069 (amended 2026-11-01): Chrome and Edge do not draw the PDF viewer in a CSP-sandboxed
/// response, so a PDF is served without `sandbox`; it keeps `nosniff` and the verified type.
#[tokio::test]
async fn a_pdf_response_has_nosniff_and_no_sandbox_csp() {
    db_test(async |pool| {
        let w = world(pool).await;
        let id = stored(&w, "a.pdf", "application/pdf", PDF).await;
        for (status, response) in all_kinds_of_answer(&w, id, PDF).await {
            assert_eq!(response.status, status);
            assert_eq!(
                response.header("x-content-type-options"),
                "nosniff",
                "{status}"
            );
            assert!(
                response.headers.get("content-security-policy").is_none(),
                "{status}: a PDF is sent without a CSP"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn every_other_file_response_has_nosniff_and_sandbox_csp() {
    db_test(async |pool| {
        let w = world(pool).await;
        let docx = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
        let cases: [(&str, &[u8], &str); 5] = [
            ("notes.txt", b"plain words", "text/plain"),
            ("pixel.png", PNG, "image/png"),
            (
                "logo.svg",
                b"<svg xmlns='x'><script/></svg>",
                "image/svg+xml",
            ),
            ("essay.docx", b"PK\x03\x04rest", docx),
            // The claim and the name say PDF, the bytes do not: stored as octet-stream.
            (
                "fake.pdf",
                b"<html><script>1</script></html>",
                "application/pdf",
            ),
        ];
        for (name, bytes, claimed) in cases {
            let id = stored(&w, name, claimed, bytes).await;
            for (status, response) in all_kinds_of_answer(&w, id, bytes).await {
                assert_eq!(response.status, status, "{name}");
                assert_eq!(
                    response.header("x-content-type-options"),
                    "nosniff",
                    "{name} {status}"
                );
                assert_eq!(
                    response.header("content-security-policy"),
                    "sandbox",
                    "{name} {status}"
                );
            }
        }
    })
    .await;
}

#[tokio::test]
async fn content_disposition_survives_unicode_and_path_names() {
    db_test(async |pool| {
        let w = world(pool).await;
        let long = format!("{}.pdf", "я".repeat(300));
        let cases: [(&str, String); 6] = [
            ("../../etc/passwd", "passwd".to_owned()),
            ("C:\\Users\\me\\Звіт.txt", "Звіт.txt".to_owned()),
            ("Звіт з фізики.pdf", "Звіт з фізики.pdf".to_owned()),
            ("a\";filename=evil.exe;.txt", "a\";filename=evil.exe;.txt".to_owned()),
            ("tab\tand\nnewline.txt", "tabandnewline.txt".to_owned()),
            (&long, format!("{}.pdf", "я".repeat(196))),
        ];
        for (raw_name, cleaned) in cases {
            let uploaded = upload(&w, raw_name, "application/octet-stream", b"bytes").await;
            assert_eq!(uploaded.status, 201, "{raw_name}");
            assert_eq!(uploaded.json()["name"], cleaned, "{raw_name}");
            let response = download(&w, &w.owner.token, id_of(&uploaded.json())).await;
            let disposition = response.header("content-disposition");
            assert!(disposition.is_ascii(), "{disposition}");
            let (fallback, extended) = disposition
                .split_once("; filename*=UTF-8''")
                .expect("both forms");
            let fallback = fallback
                .split_once("; filename=\"")
                .map(|(_, rest)| rest)
                .and_then(|rest| rest.strip_suffix('"'))
                .expect("a quoted fallback");
            assert!(
                !fallback.contains(['"', ';', '\\', '%', '\r', '\n']),
                "{fallback}"
            );
            // The extended form decodes to the cleaned name.
            let mut decoded = Vec::new();
            let bytes = extended.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'%' {
                    decoded.push(
                        u8::from_str_radix(&extended[i + 1..i + 3], 16).expect("two hex digits"),
                    );
                    i += 3;
                } else {
                    decoded.push(bytes[i]);
                    i += 1;
                }
            }
            assert_eq!(String::from_utf8(decoded).expect("UTF-8"), cleaned, "{raw_name}");
        }
        let accented = upload(&w, "Звіт з фізики.pdf", "x/y", b"b").await;
        let response = download(&w, &w.owner.token, id_of(&accented.json())).await;
        assert!(
            response
                .header("content-disposition")
                .ends_with("filename*=UTF-8''%D0%97%D0%B2%D1%96%D1%82%20%D0%B7%20%D1%84%D1%96%D0%B7%D0%B8%D0%BA%D0%B8.pdf")
        );
    })
    .await;
}

#[tokio::test]
async fn etag_is_the_sha256_and_if_none_match_gives_304() {
    db_test(async |pool| {
        let w = world(pool).await;
        let id = stored(&w, "a.pdf", "application/pdf", PDF).await;
        let etag = format!("\"{}\"", sha256(PDF));
        let first = download(&w, &w.owner.token, id).await;
        assert_eq!(first.header("etag"), etag);
        let same = get(&w, &w.owner.token, id)
            .header("if-none-match", &etag)
            .send()
            .await;
        assert_eq!(same.status, 304);
        assert!(same.body.is_empty());
        assert_eq!(same.header("etag"), etag);
        let listed = get(&w, &w.owner.token, id)
            .header("if-none-match", &format!("\"other\", W/{etag}"))
            .send()
            .await;
        assert_eq!(listed.status, 304);
        let different = get(&w, &w.owner.token, id)
            .header("if-none-match", "\"other\"")
            .send()
            .await;
        assert_eq!(different.status, 200);
        assert_eq!(different.body, PDF);
        // The tag does not let a stranger learn that the file exists.
        let stranger = get(&w, &w.stranger.token, id)
            .header("if-none-match", &etag)
            .send()
            .await;
        stranger.problem(404);
    })
    .await;
}

#[tokio::test]
async fn range_request_returns_206_with_content_range() {
    db_test(async |pool| {
        let w = world(pool).await;
        let bytes: Vec<u8> = (0..100_u8).collect();
        let id = stored(&w, "r.bin", "application/octet-stream", &bytes).await;
        let ranged = |value: &'static str| {
            let w = &w;
            async move {
                get(w, &w.owner.token, id)
                    .header("range", value)
                    .send()
                    .await
            }
        };
        let part = ranged("bytes=10-19").await;
        assert_eq!(part.status, 206);
        assert_eq!(part.header("content-range"), "bytes 10-19/100");
        assert_eq!(part.header("content-length"), "10");
        assert_eq!(part.body, &bytes[10..20]);

        let open_ended = ranged("bytes=90-").await;
        assert_eq!(open_ended.status, 206);
        assert_eq!(open_ended.header("content-range"), "bytes 90-99/100");
        assert_eq!(open_ended.body, &bytes[90..]);

        let suffix = ranged("bytes=-5").await;
        assert_eq!(suffix.status, 206);
        assert_eq!(suffix.header("content-range"), "bytes 95-99/100");
        assert_eq!(suffix.body, &bytes[95..]);

        let clamped = ranged("bytes=95-5000").await;
        assert_eq!(clamped.header("content-range"), "bytes 95-99/100");

        // A header we cannot use is ignored, as RFC 9110 says: the whole file.
        for ignored in ["bytes=abc", "bytes=5-2", "bytes=0-1,5-6", "lines=1-2"] {
            let whole = ranged(ignored).await;
            assert_eq!(whole.status, 200, "{ignored}");
            assert_eq!(whole.body, bytes);
        }
    })
    .await;
}

#[tokio::test]
async fn range_past_the_end_is_416() {
    db_test(async |pool| {
        let w = world(pool).await;
        let id = stored(&w, "r.bin", "application/octet-stream", &[1; 100]).await;
        for value in ["bytes=100-", "bytes=500-600", "bytes=-0"] {
            let response = get(&w, &w.owner.token, id)
                .header("range", value)
                .send()
                .await;
            response.problem(416);
            assert_eq!(response.header("content-range"), "bytes */100", "{value}");
        }
    })
    .await;
}

#[tokio::test]
async fn unreadable_file_is_404_not_403() {
    db_test(async |pool| {
        let w = world(pool).await;
        let pool = &w.env.pool;
        let working = stored(&w, "working.pdf", "application/pdf", PDF).await;
        name_in_item(pool, w.item, working, false).await;
        let unpublished = stored(&w, "draft.pdf", "application/pdf", PDF).await;
        name_in_item(pool, w.draft_item, unpublished, true).await;
        let shown = stored(&w, "shown.pdf", "application/pdf", PDF).await;
        name_in_item(pool, w.item, shown, true).await;

        // Students: only the published copy of a published item.
        let student = &w.student.token;
        assert_eq!(download(&w, student, shown).await.status, 200);
        download(&w, student, working).await.problem(404);
        download(&w, student, unpublished).await.problem(404);
        // Staff of the course read the working copy; nobody else reads anything.
        let owner = &w.owner.token;
        assert_eq!(download(&w, owner, working).await.status, 200);
        let stranger = &w.stranger.token;
        for id in [working, unpublished, shown] {
            download(&w, stranger, id).await.problem(404);
        }
        // An id that does not exist is the same answer as one that may not be read.
        download(&w, owner, Uuid::now_v7()).await.problem(404);
        // And a call without a login is a 401, not a hint about the file.
        let anonymous = w.env.http.get(&format!("/files/{shown}")).send().await;
        anonymous.problem(401);
    })
    .await;
}

#[tokio::test]
async fn the_openapi_document_names_both_routes() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let doc: Value = serde_json::from_slice(&env.http.get("/openapi.json").send().await.body)
            .expect("the document");
        assert!(doc["paths"]["/api/v1/files"]["post"].is_object(), "{doc}");
        assert!(
            doc["paths"]["/api/v1/files/{id}"]["get"].is_object(),
            "{doc}"
        );
        assert!(doc["components"]["schemas"]["FileRefDto"].is_object());
    })
    .await;
}
