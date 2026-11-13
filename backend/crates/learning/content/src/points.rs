//! The decimal-string form of points (ADR-0021), shared by `content` (max points) and
//! `gradebook` (scores).

use sqlx::types::Decimal;

/// Parses digits with an optional fraction of one or two digits: "20", "7.5", "20.00". Anything
/// else (a sign, an exponent, spaces, a bare dot, a third decimal, a very long number) is `None`.
/// The range is the caller's rule.
pub fn parse_decimal(raw: &str) -> Option<Decimal> {
    let (whole, fraction) = raw.split_once('.').unwrap_or((raw, ""));
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    let shape_ok = !whole.is_empty()
        && digits(whole)
        && digits(fraction)
        && (raw.contains('.') == (1..=2).contains(&fraction.len()))
        && raw.len() <= 16;
    if shape_ok { raw.parse().ok() } else { None }
}

#[cfg(test)]
mod tests;
