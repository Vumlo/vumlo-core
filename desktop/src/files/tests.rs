use std::fs;
use std::path::PathBuf;

use super::*;
use crate::core::RawReply;

const ID: &str = "0198f3a0-7b1c-7d2e-8f3a-5b6c7d8e9f01";

/// A directory of its own for a test, removed by the test at the end.
fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vumlo-files-{}-{label}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn allowlist_opens_documents_images_media_archives() {
    for ext in [
        "pdf", "doc", "docx", "odt", "rtf", "txt", "md", "ppt", "pptx", "odp", "xls", "xlsx",
        "ods", "csv", "png", "jpg", "jpeg", "gif", "webp", "mp3", "wav", "mp4", "webm", "mov",
    ] {
        assert!(opens_directly(&format!("report.{ext}")), "{ext}");
    }
    assert!(opens_directly("REPORT.PDF"));
    assert!(opens_directly("Звіт з фізики.Docx"));
}

#[test]
fn archives_are_revealed_in_their_folder_never_opened() {
    // An archive extracted by the system without a quarantine mark is the classic way in.
    for name in ["a.zip", "a.7z", "a.tar", "a.gz", "archive.tar.gz", "A.ZIP"] {
        assert!(!opens_directly(name), "{name}");
    }
}

#[test]
fn executables_and_scripts_are_never_opened() {
    for ext in [
        "exe", "bat", "cmd", "com", "sh", "app", "lnk", "scr", "msi", "jar", "desktop", "ps1",
        "vbs", "js", "EXE", "Sh",
    ] {
        assert!(!opens_directly(&format!("run.{ext}")), "{ext}");
    }
    assert!(!opens_directly("report.pdf.exe"));
    assert!(!opens_directly("report.pdf.sh"));
}

#[test]
fn unknown_extension_is_not_opened() {
    assert!(!opens_directly("notes"));
    assert!(!opens_directly("notes.xyz"));
    assert!(!opens_directly("notes."));
    assert!(!opens_directly(""));
    assert!(!opens_directly("pdf"));
}

#[test]
fn trailing_dot_or_space_after_extension_is_not_opened() {
    // Windows drops trailing dots and spaces, so `a.pdf.` and `a.pdf ` would be `a.pdf` there, and `a.exe.` is `a.exe`.
    assert!(!opens_directly("a.pdf."));
    assert!(!opens_directly("a.pdf "));
    assert!(!opens_directly("a.exe. "));
    assert!(!opens_directly("a.pdf\t"));
}

#[test]
fn unique_path_adds_a_counter_before_the_extension() {
    let dir = temp_dir("counter");
    assert_eq!(unique_path(&dir, "name.pdf"), dir.join("name.pdf"));
    fs::write(dir.join("name.pdf"), b"x").unwrap();
    assert_eq!(unique_path(&dir, "name.pdf"), dir.join("name (1).pdf"));
    fs::write(dir.join("name (1).pdf"), b"x").unwrap();
    assert_eq!(unique_path(&dir, "name.pdf"), dir.join("name (2).pdf"));
    assert_eq!(unique_path(&dir, "a.tar.gz"), dir.join("a.tar.gz"));
    fs::write(dir.join("a.tar.gz"), b"x").unwrap();
    assert_eq!(unique_path(&dir, "a.tar.gz"), dir.join("a.tar (1).gz"));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn unique_path_handles_a_name_without_extension() {
    let dir = temp_dir("noext");
    fs::write(dir.join("notes"), b"x").unwrap();
    assert_eq!(unique_path(&dir, "notes"), dir.join("notes (1)"));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn unique_path_never_overwrites_an_existing_file() {
    let dir = temp_dir("keep");
    fs::write(dir.join("a.txt"), b"first").unwrap();
    let path = unique_path(&dir, "a.txt");
    assert_ne!(path, dir.join("a.txt"));
    save_new(&dir, "a.txt", b"second").unwrap();
    save_new(&dir, "a.txt", b"third").unwrap();
    assert_eq!(fs::read(dir.join("a.txt")).unwrap(), b"first");
    assert_eq!(fs::read(dir.join("a (1).txt")).unwrap(), b"second");
    assert_eq!(fs::read(dir.join("a (2).txt")).unwrap(), b"third");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn safe_name_strips_separators_and_traversal() {
    assert_eq!(safe_name("../../etc/passwd"), "passwd");
    assert_eq!(safe_name("..\\..\\Windows\\evil.exe"), "evil.exe");
    assert_eq!(safe_name("/abs/path/report.pdf"), "report.pdf");
    assert_eq!(safe_name("a/b/"), "b");
    assert_eq!(safe_name(".."), "file");
    assert_eq!(safe_name("."), "file");
    assert_eq!(safe_name("   "), "file");
    assert_eq!(safe_name(""), "file");
    assert_eq!(safe_name("Звіт з фізики.pdf"), "Звіт з фізики.pdf");
    assert_eq!(safe_name("a:b*c?.pdf"), "a_b_c_.pdf");
}

#[test]
fn safe_name_strips_bidi_and_control_characters() {
    // A right-to-left override makes `evil\u{202e}fdp.exe` read as `evilexe.pdf`.
    assert_eq!(safe_name("evil\u{202e}fdp.exe"), "evilfdp.exe");
    assert_eq!(safe_name("a\u{200f}b\u{2066}c\u{2069}.txt"), "abc.txt");
    assert_eq!(safe_name("a\0b\r\nc\u{7f}.txt"), "abc.txt");
    assert_eq!(safe_name("\u{202e}\0"), "file");
}

#[test]
fn upload_target_grammar_rejects_bad_targets() {
    assert!(is_upload_target(&format!("item:{ID}")));
    assert!(is_upload_target(&format!("assignment:{ID}")));
    for bad in [
        "",
        "item",
        "item:",
        "item:nope",
        &format!("course:{ID}"),
        &format!("Item:{ID}"),
        &format!("item:{ID}&for=x"),
        &format!("item:{ID}:{ID}"),
        &format!("item:{ID} "),
        &format!("item:{}", ID.replace('-', "")),
        &format!("../item:{ID}"),
    ] {
        assert!(!is_upload_target(bad), "{bad}");
    }
}

#[test]
fn upload_path_encodes_the_target_and_the_decoded_name() {
    let target = format!("item:{ID}");
    let name = "%D0%97%D0%B2%D1%96%D1%82%20%26%20a%2Fb.pdf"; // Звіт & a/b.pdf
    assert_eq!(
        upload_path(&target, name).unwrap(),
        format!("/files?for=item%3A{ID}&name=%D0%97%D0%B2%D1%96%D1%82%20%26%20a%2Fb%2Epdf")
    );
    assert!(
        upload_path("item:nope", "a.pdf")
            .unwrap_err()
            .starts_with("422: ")
    );
    assert!(upload_path(&target, "").unwrap_err().starts_with("422: "));
    assert!(
        upload_path(&target, "%FF%FE")
            .unwrap_err()
            .starts_with("422: ")
    );
    assert!(
        upload_path(&target, &"a".repeat(2001))
            .unwrap_err()
            .starts_with("422: ")
    );
}

#[test]
fn vumlo_file_uri_accepts_only_uuid_paths() {
    assert_eq!(vumlo_file_id(&format!("/{ID}")), Some(ID));
    for bad in [
        "",
        "/",
        ID,
        &format!("/{ID}/"),
        &format!("/{ID}/x"),
        &format!("//{ID}"),
        &format!("/x/{ID}"),
        &format!("/../{ID}"),
        &format!("/{ID}%2e"),
        "/index.html",
        "/0198f3a0-7b1c-7d2e-8f3a-5b6c7d8e9f0",
        "/0198f3a0-7b1c-7d2e-8f3a-5b6c7d8e9f0g",
        "/0198f3a07b1c7d2e8f3a5b6c7d8e9f01",
    ] {
        assert_eq!(vumlo_file_id(bad), None, "{bad}");
    }
}

#[test]
fn the_scheme_answers_only_to_the_app_itself() {
    for ok in [
        "tauri://localhost",
        "http://tauri.localhost",
        "https://tauri.localhost",
    ] {
        assert!(own_origin(ok), "{ok}");
    }
    for bad in [
        "https://www.youtube-nocookie.com",
        "null",
        "http://tauri.localhost.evil.com",
        "",
    ] {
        assert!(!own_origin(bad), "{bad}");
    }
}

#[test]
fn the_name_comes_from_the_filename_star_of_content_disposition() {
    let header = "attachment; filename=\"____ _.pdf\"; filename*=UTF-8''%D0%97%D0%B2%D1%96%D1%82%20a%26b.pdf";
    assert_eq!(disposition_name(header).as_deref(), Some("Звіт a&b.pdf"));
    assert_eq!(
        disposition_name("inline; filename=\"plain.png\"").as_deref(),
        Some("plain.png")
    );
    assert_eq!(disposition_name("attachment"), None);
}

#[test]
fn a_refused_request_carries_the_status_and_the_problem_detail() {
    let body = br#"{"title":"Unprocessable","detail":"This file type is not allowed."}"#;
    assert_eq!(error_text(422, body), "422: This file type is not allowed.");
    assert_eq!(
        error_text(413, br#"{"title":"Payload Too Large"}"#),
        "413: Payload Too Large"
    );
    assert_eq!(error_text(502, b"<html>"), "502: ");
}

#[test]
fn an_outcome_is_written_in_snake_case() {
    assert_eq!(
        serde_json::to_string(&OpenOutcome::Opened).unwrap(),
        "\"opened\""
    );
    assert_eq!(
        serde_json::to_string(&OpenOutcome::Revealed).unwrap(),
        "\"revealed\""
    );
    assert_eq!(
        serde_json::to_string(&OpenOutcome::SavedOnly).unwrap(),
        "\"saved_only\""
    );
}

// --- the fix round ---------------------------------------------------------------------------------------

#[test]
fn a_name_with_dots_in_a_row_is_encoded_so_the_path_stays_valid() {
    let target = format!("item:{ID}");
    // What the window sends is `encodeURIComponent` of the name, which leaves dots alone.
    for (sent, encoded) in [
        ("Lab%201...pdf", "Lab%201%2E%2E%2Epdf"),
        ("v1..2.docx", "v1%2E%2E2%2Edocx"),
    ] {
        let path = upload_path(&target, sent).unwrap();
        assert!(path.ends_with(&format!("&name={encoded}")), "{path}");
        assert!(!path.contains(".."), "{path}");
        assert!(crate::core::valid_path(&path), "{path}");
    }
    // A traversal in the part before the query is still refused.
    assert!(!crate::core::valid_path("/files/../me"));
}

#[test]
fn a_long_name_is_cut_at_a_character_and_keeps_its_extension() {
    let long = format!("{}.pdf", "ж".repeat(150)); // 300 bytes
    let name = safe_name(&long);
    assert!(name.len() <= NAME_BYTES - COUNTER_BYTES, "{}", name.len());
    assert!(name.ends_with(".pdf"));
    assert!(name.starts_with('ж') && name.chars().count() > 50);
    assert!(opens_directly(&name));
    // Ascii: 200 characters at most.
    let ascii = safe_name(&format!("{}.txt", "a".repeat(500)));
    assert!(ascii.chars().count() <= 200 && ascii.ends_with(".txt"));
    // A short name is left alone, and an absurd extension does not keep the name from being cut.
    assert_eq!(safe_name("short.pdf"), "short.pdf");
    let odd = safe_name(&format!("x.{}", "e".repeat(400)));
    assert!(odd.len() <= NAME_BYTES - COUNTER_BYTES);
}

#[test]
fn a_long_name_still_fits_with_a_counter() {
    let dir = temp_dir("long");
    let name = safe_name(&format!("{}.pdf", "ж".repeat(150)));
    save_new(&dir, &name, b"1").unwrap();
    let second = save_new(&dir, &name, b"2").unwrap();
    let file = second.file_name().unwrap().to_str().unwrap();
    assert!(file.contains(" (1)") && file.ends_with(".pdf"));
    assert!(file.len() <= NAME_BYTES, "{}", file.len());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn what_the_server_says_about_a_file_is_forwarded_as_it_came() {
    let reply = |headers: &[(&str, &str)]| RawReply {
        status: 200,
        headers: headers
            .iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect(),
        body: Vec::new(),
    };
    let sent = reply(&[
        ("content-type", "application/pdf"),
        ("content-security-policy", "sandbox"),
        ("x-content-type-options", "nosniff"),
        ("set-cookie", "a=b"),
        ("server", "x"),
    ]);
    let forwarded = file_headers(&sent);
    assert!(forwarded.contains(&("content-type", "application/pdf")));
    assert!(forwarded.contains(&("content-security-policy", "sandbox")));
    assert_eq!(
        forwarded
            .iter()
            .filter(|(n, _)| *n == "x-content-type-options")
            .count(),
        1
    );
    assert!(
        !forwarded
            .iter()
            .any(|(n, _)| *n == "set-cookie" || *n == "server")
    );
    // The server is not made to say what it did not: no policy is invented, but nosniff is never missing.
    let plain = reply(&[("content-type", "image/png")]);
    let bare = file_headers(&plain);
    assert!(!bare.iter().any(|(n, _)| *n == "content-security-policy"));
    assert!(bare.contains(&("x-content-type-options", "nosniff")));
}

#[test]
fn the_client_type_is_a_well_formed_media_type_or_octet_stream() {
    assert_eq!(upload_mime(Some("application/pdf")), "application/pdf");
    assert_eq!(
        upload_mime(Some(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        )),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    );
    assert_eq!(upload_mime(Some("text/plain; charset=utf-8")), "text/plain");
    assert_eq!(upload_mime(Some("image/svg+xml")), "image/svg+xml");
    for bad in [
        "",
        "pdf",
        "/",
        "a/",
        "/b",
        "a/b/c",
        "a b/c",
        "text/pl\\ain",
        "a/b\r\nx: y",
        "a/\u{43b}",
        &"a/".repeat(200),
    ] {
        assert_eq!(upload_mime(Some(bad)), "application/octet-stream", "{bad}");
    }
    assert_eq!(upload_mime(None), "application/octet-stream");
}

#[test]
fn a_saved_file_is_marked_as_coming_from_the_internet() {
    // Windows reads this stream (`<file>:Zone.Identifier`) to warn before it runs or unpacks the file; 3 = Internet.
    assert_eq!(zone_identifier(), "[ZoneTransfer]\r\nZoneId=3\r\n");
}

/// The page reads the name of a file from `Content-Disposition`, so a cross-origin `fetch` must be allowed to.
#[tokio::test]
async fn the_page_may_read_the_content_disposition_of_a_file() {
    let request = http::Request::builder()
        .method("GET")
        .uri(format!("vumlo-file://localhost/{ID}"))
        .header("origin", "tauri://localhost")
        .body(Vec::new())
        .unwrap();
    // No core is running: the refusal carries the same CORS headers as an answer.
    let response = serve_one(None, &request).await;
    let exposed = response.headers()["access-control-expose-headers"]
        .to_str()
        .unwrap();
    for name in [
        "Content-Range",
        "Content-Length",
        "Content-Type",
        "Content-Disposition",
    ] {
        assert!(exposed.contains(name), "{name} in {exposed}");
    }
}
