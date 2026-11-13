//! Content-type sniffing and file-name cleaning for uploads (ADR-0069). Pure functions: the six
//! inline types are decided by the bytes, never by the client's claim or the name.

const OCTET_STREAM: &str = "application/octet-stream";
const INLINE: [&str; 6] = [
    "application/pdf",
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "text/plain",
];
const MAX_NAME_CHARS: usize = 200;

/// Extension (lowercase) to type. `svg` is here on purpose: it is never inline, so it downloads.
const BY_EXTENSION: [(&str, &str); 20] = [
    ("pdf", "application/pdf"),
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("txt", "text/plain"),
    ("svg", "image/svg+xml"),
    (
        "docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    (
        "pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ),
    (
        "xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
    ("odt", "application/vnd.oasis.opendocument.text"),
    ("odp", "application/vnd.oasis.opendocument.presentation"),
    ("ods", "application/vnd.oasis.opendocument.spreadsheet"),
    ("zip", "application/zip"),
    ("mp3", "audio/mpeg"),
    ("mp4", "video/mp4"),
    ("md", "text/markdown"),
    ("csv", "text/csv"),
    ("json", "application/json"),
];

/// True for the six types that may be shown in the browser (`Content-Disposition: inline`).
pub fn is_inline(mime: &str) -> bool {
    INLINE.contains(&mime)
}

/// True for the four image types an `image` block may show.
pub fn is_image(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

/// The type to store for an upload. `head` is the first bytes of the file. A type that may be shown
/// inline is given only when the bytes agree; otherwise the claim and the name are hints at most.
pub fn sniff(name: &str, head: &[u8], claimed: &str) -> String {
    if let Some(found) = signature(head) {
        return found.to_owned();
    }
    let claimed = claimed
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let hint = extension_type(name).or_else(|| well_formed(&claimed).then_some(claimed.as_str()));
    match hint {
        Some("text/plain") if is_text(head) => "text/plain".to_owned(),
        Some(other) if !is_inline(other) => other.to_owned(),
        _ => OCTET_STREAM.to_owned(),
    }
}

fn signature(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"%PDF-") {
        Some("application/pdf")
    } else if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if head.starts_with(b"\xFF\xD8\xFF") {
        Some("image/jpeg")
    } else if head.starts_with(b"GIF8") {
        Some("image/gif")
    } else if head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

fn extension_type(name: &str) -> Option<&'static str> {
    let (_, ext) = name.rsplit_once('.')?;
    let ext = ext.to_ascii_lowercase();
    BY_EXTENSION
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, t)| *t)
}

/// A claimed type goes into a header later; accept only `type/subtype` made of token characters.
fn well_formed(claimed: &str) -> bool {
    claimed.len() <= 100
        && claimed.split_once('/').is_some_and(|(a, b)| {
            [a, b].iter().all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"+-.".contains(&c))
            })
        })
}

/// Valid UTF-8 without NUL. The head may end inside a multi-byte character, which is not an error.
fn is_text(head: &[u8]) -> bool {
    if head.contains(&0) {
        return false;
    }
    match std::str::from_utf8(head) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none(),
    }
}

/// The last path component of `raw` (split on `/` and `\`), without control characters, trimmed,
/// at most 200 characters with the extension kept. Never empty, never `.` or `..`.
pub fn clean_name(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    // Bidi marks and overrides (category Cf) would let `fdp.exe` render as `exe.pdf`. U+200C and
    // U+200D stay: Persian, Indic scripts and emoji sequences need them.
    let cleaned: String = last
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
        })
        .collect();
    let name = cleaned.trim();
    if matches!(name, "" | "." | "..") {
        return "file".to_owned();
    }
    if name.chars().count() <= MAX_NAME_CHARS {
        return name.to_owned();
    }
    // A "extension" longer than 20 characters is part of the name, not an extension.
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e)
        .filter(|e| e.chars().count() <= 20)
        .map_or(String::new(), |e| format!(".{e}"));
    let keep = MAX_NAME_CHARS - ext.chars().count();
    let stem: String = name.chars().take(keep).collect();
    format!("{}{ext}", stem.trim_end())
}

#[cfg(test)]
mod tests;
