use std::fmt;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sqlx::Postgres;
use sqlx::postgres::{PgTypeInfo, PgValueRef};
use sqlx::types::Decimal;
use uuid::Uuid;

use crate::policy::Release;
use crate::text_enum::text_enum;

const TITLE_MAX_LEN: usize = 200;

/// A value that failed validation. Each variant maps to one field of the API.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    #[error("must be 1 to {TITLE_MAX_LEN} characters")]
    Title,
    #[error("must be a number above 0 and at most 9999.99, with at most two decimals")]
    MaxPoints,
    #[error("unknown assignment type; available: text")]
    AssignmentType,
    #[error("{0}")]
    Config(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentTitle(String);

impl ContentTitle {
    pub fn parse(raw: &str) -> Result<Self, ModelError> {
        let title = raw.trim();
        let len = title.chars().count();
        if (1..=TITLE_MAX_LEN).contains(&len) && !title.chars().any(char::is_control) {
            Ok(Self(title.to_owned()))
        } else {
            Err(ModelError::Title)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleStatus {
    Draft,
    Published,
}
text_enum!(ModuleStatus { Draft => "draft", Published => "published" });

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    Draft,
    Published,
}
text_enum!(ItemStatus { Draft => "draft", Published => "published" });

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Lesson,
    Assignment,
}
text_enum!(ItemKind { Lesson => "lesson", Assignment => "assignment" });

/// The most points one assignment can be worth (`numeric(6,2)`, ADR-0021).
const MAX_POINTS_LIMIT: Decimal = Decimal::from_parts(999_999, 0, 0, false, 2);

/// What an assignment is worth: above 0, at most 9999.99, at most two decimals. It knows nothing
/// of the grading scale (ADR-0022); the service applies that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaxPoints(Decimal);

impl MaxPoints {
    /// Accepts digits with an optional fraction of one or two digits: "20", "7.5", "20.00".
    pub fn parse(raw: &str) -> Result<Self, ModelError> {
        let (whole, fraction) = raw.split_once('.').unwrap_or((raw, ""));
        let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
        let shape_ok = !whole.is_empty()
            && digits(whole)
            && digits(fraction)
            && (raw.contains('.') == (1..=2).contains(&fraction.len()))
            && raw.len() <= 16;
        if !shape_ok {
            return Err(ModelError::MaxPoints);
        }
        let value: Decimal = raw.parse().map_err(|_| ModelError::MaxPoints)?;
        Self::from_decimal(value)
    }

    fn from_decimal(value: Decimal) -> Result<Self, ModelError> {
        if value > Decimal::ZERO && value <= MAX_POINTS_LIMIT && value.scale() <= 2 {
            Ok(Self(value))
        } else {
            Err(ModelError::MaxPoints)
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

/// What an assignment type must provide (ADR-0008). It is the future plugin boundary, so it stays
/// small: submissions and auto-grading join it with their own tasks.
pub trait AssignmentType {
    /// `Ok` when `config` is a valid configuration for this type.
    fn validate_config(&self, config: &Map<String, Value>) -> Result<(), ModelError>;
}

/// The types built into the server. The database column is plain text (ADR-0008).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinType {
    Text,
}
text_enum!(BuiltinType { Text => "text" });

impl BuiltinType {
    /// An unknown or not-yet-built type is an error that does not repeat the input.
    pub fn parse(raw: &str) -> Result<Self, ModelError> {
        raw.parse().map_err(|_| ModelError::AssignmentType)
    }
}

impl AssignmentType for BuiltinType {
    fn validate_config(&self, config: &Map<String, Value>) -> Result<(), ModelError> {
        match self {
            // A text answer needs no settings; any field is a mistake.
            Self::Text => match config.keys().next() {
                None => Ok(()),
                Some(field) => Err(ModelError::Config(format!(
                    "unknown field `{field}`: the text type has no settings"
                ))),
            },
        }
    }
}

/// The assignment part of an item of kind `assignment`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub assignment_type: BuiltinType,
    pub config: Map<String, Value>,
    pub max_points: MaxPoints,
    pub is_bonus: bool,
}

/// What the course tree shows of an assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignmentSummary {
    pub assignment_type: BuiltinType,
    pub max_points: MaxPoints,
    pub is_bonus: bool,
}

/// A top-level module (`parent_id` is `None`) or a sub-module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub id: Uuid,
    pub course_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub position: i32,
    pub title: String,
    pub status: ModuleStatus,
    pub publish_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Module {
    pub fn release(&self) -> Release {
        Release {
            published: self.status == ModuleStatus::Published,
            publish_at: self.publish_at,
        }
    }
}

/// A lesson or an assignment. `body` is a JSON object the server never interprets (ADR-0020); for
/// an assignment it holds the instructions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: Uuid,
    pub module_id: Uuid,
    pub course_id: Uuid,
    pub position: i32,
    pub kind: ItemKind,
    pub title: String,
    pub body: Map<String, Value>,
    pub status: ItemStatus,
    pub publish_at: Option<DateTime<Utc>>,
    /// Present exactly for items of kind `assignment`.
    pub assignment: Option<Assignment>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Item {
    pub fn release(&self) -> Release {
        Release {
            published: self.status == ItemStatus::Published,
            publish_at: self.publish_at,
        }
    }
}

/// An item as the course tree lists it: no body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemSummary {
    pub id: Uuid,
    pub kind: ItemKind,
    pub title: String,
    pub status: ItemStatus,
    pub publish_at: Option<DateTime<Utc>>,
    pub position: i32,
    pub assignment: Option<AssignmentSummary>,
}

impl ItemSummary {
    pub fn release(&self) -> Release {
        Release {
            published: self.status == ItemStatus::Published,
            publish_at: self.publish_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentSubModule {
    pub module: Module,
    pub items: Vec<ItemSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentModule {
    pub module: Module,
    pub submodules: Vec<ContentSubModule>,
    pub items: Vec<ItemSummary>,
}

/// A change to a module. `publish_at`: `None` leaves it, `Some(None)` clears it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModulePatch {
    pub title: Option<ContentTitle>,
    pub publish_at: Option<Option<DateTime<Utc>>>,
}

impl ModulePatch {
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.publish_at.is_none()
    }
}

/// A change to an item. `body` replaces the whole body when set; `config`, `max_points` and
/// `is_bonus` apply to assignments only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemPatch {
    pub title: Option<ContentTitle>,
    pub body: Option<Map<String, Value>>,
    pub publish_at: Option<Option<DateTime<Utc>>>,
    pub config: Option<Map<String, Value>>,
    pub max_points: Option<MaxPoints>,
    pub is_bonus: Option<bool>,
}

impl ItemPatch {
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.body.is_none()
            && self.publish_at.is_none()
            && !self.touches_assignment()
    }

    /// True when the patch sets a field that lives on the assignment row.
    pub fn touches_assignment(&self) -> bool {
        self.config.is_some() || self.max_points.is_some() || self.is_bonus.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_is_trimmed_and_bounded() {
        assert_eq!(
            ContentTitle::parse("  Lectures ").unwrap().as_str(),
            "Lectures"
        );
        assert!(ContentTitle::parse("").is_err());
        assert!(ContentTitle::parse("   ").is_err());
        assert!(ContentTitle::parse("a\nb").is_err());
        assert!(ContentTitle::parse("a\u{7f}b").is_err());
        assert!(ContentTitle::parse(&"ł".repeat(200)).is_ok());
        assert!(ContentTitle::parse(&"ł".repeat(201)).is_err());
    }

    #[test]
    fn enums_round_trip() {
        for status in [ModuleStatus::Draft, ModuleStatus::Published] {
            assert_eq!(status.as_str().parse::<ModuleStatus>(), Ok(status));
        }
        for status in [ItemStatus::Draft, ItemStatus::Published] {
            assert_eq!(status.as_str().parse::<ItemStatus>(), Ok(status));
        }
        assert_eq!("lesson".parse::<ItemKind>(), Ok(ItemKind::Lesson));
        assert_eq!("assignment".parse::<ItemKind>(), Ok(ItemKind::Assignment));
        assert!("quiz".parse::<ItemKind>().is_err());
    }

    #[test]
    fn max_points_accepts_decimal_strings_up_to_two_decimals() {
        for ok in ["20", "7.5", "20.00", "0.01", "9999.99", "9999", "0012"] {
            assert!(MaxPoints::parse(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn max_points_rejects_everything_else() {
        for bad in [
            "0",
            "0.00",
            "-1",
            "10000",
            "9999.991",
            "10000.00",
            "1.005",
            "abc",
            "",
            ".5",
            "5.",
            "+5",
            "1e2",
            " 5",
            "5 ",
            "1,5",
            "1.2.3",
            "99999999999999999999",
        ] {
            assert_eq!(MaxPoints::parse(bad), Err(ModelError::MaxPoints), "{bad:?}");
        }
    }

    #[test]
    fn max_points_display_has_exactly_two_decimals() {
        for (raw, shown) in [
            ("20", "20.00"),
            ("7.5", "7.50"),
            ("20.00", "20.00"),
            ("0.01", "0.01"),
            ("9999.99", "9999.99"),
        ] {
            assert_eq!(MaxPoints::parse(raw).unwrap().to_string(), shown);
        }
    }

    #[test]
    fn the_type_enum_parses_text_and_nothing_else() {
        assert_eq!(BuiltinType::parse("text"), Ok(BuiltinType::Text));
        assert_eq!(BuiltinType::Text.as_str(), "text");
        for bad in ["form", "", "Text", "no_submission"] {
            assert_eq!(
                BuiltinType::parse(bad),
                Err(ModelError::AssignmentType),
                "{bad:?}"
            );
        }
        assert!(!ModelError::AssignmentType.to_string().contains("form"));
    }

    #[test]
    fn the_text_type_takes_an_empty_config_only() {
        let mut config = Map::new();
        assert_eq!(BuiltinType::Text.validate_config(&config), Ok(()));
        config.insert("limit".to_owned(), Value::from(5));
        let error = BuiltinType::Text.validate_config(&config).unwrap_err();
        assert!(error.to_string().contains("limit"), "{error}");
    }

    #[test]
    fn empty_patches_are_recognised() {
        assert!(ModulePatch::default().is_empty());
        assert!(ItemPatch::default().is_empty());
        let sets_points = ItemPatch {
            max_points: Some(MaxPoints::parse("5").unwrap()),
            ..ItemPatch::default()
        };
        assert!(!sets_points.is_empty());
        assert!(sets_points.touches_assignment());
        let clears = ModulePatch {
            publish_at: Some(None),
            ..ModulePatch::default()
        };
        assert!(!clears.is_empty());
    }
}
