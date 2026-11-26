//! The decimal-string form of points (ADR-0021), shared by `content` (max points), `gradebook`
//! (scores) and the types whose questions carry points (ADR-0050).

use std::fmt;

use sqlx::Postgres;
use sqlx::postgres::{PgTypeInfo, PgValueRef};
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

/// The most points one assignment can be worth (`numeric(6,2)`, ADR-0021).
const MAX_POINTS_LIMIT: Decimal = Decimal::from_parts(999_999, 0, 0, false, 2);

/// A value that is not a valid number of points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("must be a number above 0 and at most 9999.99, with at most two decimals")]
pub struct PointsError;

/// What an assignment is worth: above 0, at most 9999.99, at most two decimals. It knows nothing
/// of the grading scale (ADR-0022); the service applies that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaxPoints(Decimal);

impl MaxPoints {
    /// Accepts digits with an optional fraction of one or two digits: "20", "7.5", "20.00".
    pub fn parse(raw: &str) -> Result<Self, PointsError> {
        let value = parse_decimal(raw).ok_or(PointsError)?;
        Self::from_decimal(value)
    }

    fn from_decimal(value: Decimal) -> Result<Self, PointsError> {
        if value > Decimal::ZERO && value <= MAX_POINTS_LIMIT && value.scale() <= 2 {
            Ok(Self(value))
        } else {
            Err(PointsError)
        }
    }

    pub fn value(self) -> Decimal {
        self.0
    }
}

impl fmt::Display for MaxPoints {
    /// Always two decimals: "20.00".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.2}", self.0)
    }
}

impl sqlx::Type<Postgres> for MaxPoints {
    fn type_info() -> PgTypeInfo {
        <Decimal as sqlx::Type<Postgres>>::type_info()
    }
    fn compatible(ty: &PgTypeInfo) -> bool {
        <Decimal as sqlx::Type<Postgres>>::compatible(ty)
    }
}

impl<'r> sqlx::Decode<'r, Postgres> for MaxPoints {
    fn decode(value: PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let decimal = <Decimal as sqlx::Decode<Postgres>>::decode(value)?;
        Ok(Self::from_decimal(decimal)?)
    }
}

#[cfg(test)]
mod tests;
