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
mod tests {
    use super::*;

    #[test]
    fn accepts_up_to_two_decimals() {
        for ok in ["0", "0.00", "20", "7.5", "20.00", "0.01", "9999.99", "0012"] {
            assert!(parse_decimal(ok).is_some(), "{ok}");
        }
    }

    #[test]
    fn rejects_every_other_shape() {
        for bad in [
            "",
            ".5",
            "5.",
            "+5",
            "-1",
            "1e2",
            " 5",
            "5 ",
            "1,5",
            "1.2.3",
            "1.005",
            "abc",
            "99999999999999999999",
        ] {
            assert!(parse_decimal(bad).is_none(), "{bad:?}");
        }
    }
}
