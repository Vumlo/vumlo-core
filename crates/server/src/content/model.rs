use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::policy::Release;
use crate::text_enum::text_enum;

const TITLE_MAX_LEN: usize = 200;

/// A value that failed validation. Each variant maps to one field of the API.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    #[error("must be 1 to {TITLE_MAX_LEN} characters")]
    Title,
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

/// Only lessons exist so far (ADR-0020); assignments arrive with their own task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Lesson,
}
text_enum!(ItemKind { Lesson => "lesson" });

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

/// A lesson. `body` is a JSON object the server never interprets (ADR-0020).
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

/// A change to an item. `body` replaces the whole body when set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemPatch {
    pub title: Option<ContentTitle>,
    pub body: Option<Map<String, Value>>,
    pub publish_at: Option<Option<DateTime<Utc>>>,
}

impl ItemPatch {
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.body.is_none() && self.publish_at.is_none()
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
        assert!("assignment".parse::<ItemKind>().is_err());
    }

    #[test]
    fn empty_patches_are_recognised() {
        assert!(ModulePatch::default().is_empty());
        assert!(ItemPatch::default().is_empty());
        let clears = ModulePatch {
            publish_at: Some(None),
            ..ModulePatch::default()
        };
        assert!(!clears.is_empty());
    }
}
