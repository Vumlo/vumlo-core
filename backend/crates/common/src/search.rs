//! What a "find people" search shares: the text pattern and the limits (ADR-0048, ADR-0094).

/// The most candidates a search returns.
pub const CANDIDATE_LIMIT: i64 = 20;
/// The shortest search text.
pub const CANDIDATE_MIN_CHARS: usize = 2;

/// Escapes `\`, `%` and `_` so that the text is matched as it is.
pub fn like_pattern(text: &str) -> String {
    let mut pattern = String::with_capacity(text.len() + 2);
    pattern.push('%');
    for c in text.chars() {
        if matches!(c, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    pattern
}

#[cfg(test)]
mod tests;
