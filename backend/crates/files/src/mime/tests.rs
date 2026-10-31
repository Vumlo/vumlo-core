use super::*;

#[test]
fn sniff_accepts_real_pdf_png_jpeg_gif_webp() {
    assert_eq!(sniff("a.bin", b"%PDF-1.7 rest", ""), "application/pdf");
    assert_eq!(sniff("a.bin", b"\x89PNG\r\n\x1a\nrest", ""), "image/png");
    assert_eq!(sniff("a.bin", b"\xFF\xD8\xFF\xE0rest", ""), "image/jpeg");
    assert_eq!(sniff("a.bin", b"GIF89arest", ""), "image/gif");
    assert_eq!(sniff("a.bin", b"RIFF\x24\0\0\0WEBPVP8 ", ""), "image/webp");
    // The bytes decide, whatever the name or the claim says.
    assert_eq!(sniff("a.txt", b"%PDF-1.7", "text/plain"), "application/pdf");
}

#[test]
fn sniff_demotes_a_fake_pdf_to_octet_stream() {
    assert_eq!(
        sniff("a.pdf", b"hello", "application/pdf"),
        "application/octet-stream"
    );
    assert_eq!(sniff("a.png", b"hello", ""), "application/octet-stream");
    assert_eq!(
        sniff("a.bin", b"hello", "image/jpeg"),
        "application/octet-stream"
    );
    // A RIFF container that is not WebP.
    assert_eq!(
        sniff("a.webp", b"RIFF\x24\0\0\0WAVEfmt ", ""),
        "application/octet-stream"
    );
}

#[test]
fn sniff_treats_svg_as_attachment_type() {
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>";
    for t in [
        sniff("a.svg", svg, "image/svg+xml"),
        sniff("a.svg", svg, ""),
        sniff("a.bin", svg, "image/svg+xml"),
    ] {
        assert_eq!(t, "image/svg+xml");
        assert!(!is_inline(&t));
    }
}

#[test]
fn sniff_plain_text_requires_valid_utf8_without_nul() {
    assert_eq!(sniff("a.txt", b"hello", "text/plain"), "text/plain");
    assert_eq!(sniff("a.txt", "Привіт".as_bytes(), ""), "text/plain");
    assert_eq!(sniff("a.bin", b"hello", "text/plain"), "text/plain");
    assert_eq!(sniff("a.txt", b"he\0llo", ""), "application/octet-stream");
    assert_eq!(
        sniff("a.txt", b"\xff\xfe\xfd", ""),
        "application/octet-stream"
    );
    // The 512-byte head may cut a multi-byte character at the end.
    let cut = &"Привіт".as_bytes()[..5];
    assert_eq!(sniff("a.txt", cut, ""), "text/plain");
    // A bad byte in the middle is not a cut.
    assert_eq!(sniff("a.txt", b"a\xd0b", ""), "application/octet-stream");
}

#[test]
fn unknown_extension_keeps_a_guess_from_the_extension_table() {
    let zip = b"PK\x03\x04rest";
    for (name, want) in [
        (
            "a.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
        (
            "a.pptx",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ),
        (
            "a.xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
        ("a.odt", "application/vnd.oasis.opendocument.text"),
        ("a.odp", "application/vnd.oasis.opendocument.presentation"),
        ("a.ods", "application/vnd.oasis.opendocument.spreadsheet"),
        ("a.zip", "application/zip"),
        ("a.mp4", "video/mp4"),
        ("a.mp3", "audio/mpeg"),
        ("a.csv", "text/csv"),
        ("a.md", "text/markdown"),
        ("a.json", "application/json"),
        (
            "A.DOCX",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
    ] {
        let got = sniff(name, zip, "");
        assert_eq!(got, want, "{name}");
        assert!(!is_inline(&got), "{name}");
    }
    // The claim is only a hint for a name the table does not know.
    assert_eq!(
        sniff("a.xyz", zip, "application/x-foo"),
        "application/x-foo"
    );
    assert_eq!(sniff("a.xyz", zip, ""), "application/octet-stream");
    assert_eq!(
        sniff("a", zip, "not a type\r\n"),
        "application/octet-stream"
    );
}

#[test]
fn only_the_six_types_are_inline() {
    for t in [
        "application/pdf",
        "image/png",
        "image/jpeg",
        "image/gif",
        "image/webp",
        "text/plain",
    ] {
        assert!(is_inline(t), "{t}");
    }
    for t in [
        "image/svg+xml",
        "text/html",
        "text/csv",
        "application/octet-stream",
        "",
    ] {
        assert!(!is_inline(t), "{t}");
    }
}

#[test]
fn clean_name_strips_path_parts_and_controls() {
    assert_eq!(clean_name("../../etc/passwd"), "passwd");
    assert_eq!(clean_name("a\\b/c.pdf"), "c.pdf");
    assert_eq!(clean_name("C:\\Users\\me\\r.pdf"), "r.pdf");
    assert_eq!(clean_name("  re\tport\r\n\0.pdf  "), "report.pdf");
}

#[test]
fn clean_name_keeps_unicode() {
    assert_eq!(clean_name("Звіт з фізики.pdf"), "Звіт з фізики.pdf");
}

#[test]
fn clean_name_truncates_to_200_chars_keeping_the_extension() {
    let long = format!("{}.pdf", "я".repeat(300));
    let got = clean_name(&long);
    assert_eq!(got.chars().count(), 200);
    assert!(got.ends_with(".pdf"));
    assert!(got.starts_with("яяя"));
    let no_ext = "b".repeat(300);
    assert_eq!(clean_name(&no_ext).chars().count(), 200);
    let exact = format!("{}.pdf", "c".repeat(196));
    assert_eq!(clean_name(&exact), exact);
}

#[test]
fn clean_name_never_returns_empty() {
    for raw in ["", "   ", "\0\r\n", "..", ".", "a/..", "dir/", "\\"] {
        assert_eq!(clean_name(raw), "file", "{raw:?}");
    }
}
