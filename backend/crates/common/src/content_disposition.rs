//! The `Content-Disposition` header of a download (RFC 6266), shared by every route that sends a
//! file: an uploaded one and the exported workbook of the gradebook.

use axum::http::HeaderValue;

/// `inline` or `attachment`, with the name twice (RFC 6266): `filename*` carries it in UTF-8
/// (RFC 8187) and `filename` is an ASCII fallback in which nothing can end the quoted string or
/// start a parameter.
pub fn content_disposition(name: &str, inline: bool) -> HeaderValue {
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
