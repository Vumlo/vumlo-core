use std::fs;
use std::path::PathBuf;

use super::*;

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
        "zip", "7z", "tar", "gz",
    ] {
        assert!(opens_directly(&format!("report.{ext}")), "{ext}");
    }
    assert!(opens_directly("REPORT.PDF"));
    assert!(opens_directly("Звіт з фізики.Docx"));
    assert!(opens_directly("archive.tar.gz"));
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
        format!("/files?for=item%3A{ID}&name=%D0%97%D0%B2%D1%96%D1%82%20%26%20a%2Fb.pdf")
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
