use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::policy::{CourseRole, CourseStatus};
use crate::text_enum::text_enum;

const TITLE_MAX_LEN: usize = 200;

/// A value that failed validation. Each variant maps to one field of the API.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    #[error("must be 1 to {TITLE_MAX_LEN} characters")]
    Title,
}

/// ADR-0005: templates share the table. Only regular courses can be created through the API yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CourseKind {
    Regular,
    Template,
}
text_enum!(CourseKind { Regular => "regular", Template => "template" });

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CourseTitle(String);

impl CourseTitle {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Course {
    pub id: Uuid,
    pub kind: CourseKind,
    pub title: String,
    pub status: CourseStatus,
    pub owner_id: Uuid,
    pub term_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Membership {
    pub course_id: Uuid,
    pub user_id: Uuid,
    pub role: CourseRole,
    pub is_owner: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_is_trimmed_and_bounded() {
        assert_eq!(
            CourseTitle::parse("  Algebra 7-A ").unwrap().as_str(),
            "Algebra 7-A"
        );
        assert!(CourseTitle::parse("").is_err());
        assert!(CourseTitle::parse("   ").is_err());
        assert!(CourseTitle::parse("a\nb").is_err());
        assert!(CourseTitle::parse(&"ł".repeat(200)).is_ok());
        assert!(CourseTitle::parse(&"ł".repeat(201)).is_err());
    }

    #[test]
    fn course_kind_round_trips() {
        for kind in [CourseKind::Regular, CourseKind::Template] {
            assert_eq!(kind.as_str().parse::<CourseKind>(), Ok(kind));
        }
        assert!("course".parse::<CourseKind>().is_err());
    }
}
