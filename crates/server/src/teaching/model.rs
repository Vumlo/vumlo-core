use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::policy::{CourseRole, CourseStatus};
use crate::text_enum::text_enum;

const TITLE_MAX_LEN: usize = 200;
const CODE_MAX_LEN: usize = 32;
const DESCRIPTION_MAX_LEN: usize = 300;

/// A value that failed validation. Each variant maps to one field of the API.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    #[error("must be 1 to {TITLE_MAX_LEN} characters")]
    Title,
    #[error("must be at most {CODE_MAX_LEN} characters")]
    Code,
    #[error("must be at most {DESCRIPTION_MAX_LEN} characters")]
    Description,
}

/// A course's short code ("CS 305"): trimmed, at most 32 characters, empty for none (ADR-0048).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CourseCode(String);

impl CourseCode {
    pub fn parse(raw: &str) -> Result<Self, ModelError> {
        let code = raw.trim();
        if code.chars().count() <= CODE_MAX_LEN && !code.chars().any(char::is_control) {
            Ok(Self(code.to_owned()))
        } else {
            Err(ModelError::Code)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A course's description: trimmed, at most 300 characters, line breaks allowed, empty for none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CourseDescription(String);

impl CourseDescription {
    pub fn parse(raw: &str) -> Result<Self, ModelError> {
        let text = raw.trim();
        if text.chars().count() <= DESCRIPTION_MAX_LEN
            && !text.chars().any(|c| c.is_control() && c != '\n')
        {
            Ok(Self(text.to_owned()))
        } else {
            Err(ModelError::Description)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The fields of a course-info update that were sent.
#[derive(Debug, Clone, Default)]
pub struct CourseInfoUpdate {
    pub code: Option<String>,
    pub description: Option<String>,
}

/// The header of a course (ADR-0048).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CourseInfo {
    pub course_id: Uuid,
    pub code: String,
    pub description: String,
    pub term_name: Option<String>,
    pub owner_name: String,
    pub owner_surname: String,
    pub owner_email: String,
    pub student_count: i64,
    pub former_count: i64,
    pub assistant_count: i64,
    pub updated_at: DateTime<Utc>,
    pub content_updated_at: Option<DateTime<Utc>>,
    pub total_points: sqlx::types::Decimal,
    pub graded_items: i64,
    pub has_grades: bool,
}

/// A template and what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateSummary {
    pub course: Course,
    pub sections: i64,
    pub lectures: i64,
    pub assignments: i64,
    pub updated_at: DateTime<Utc>,
}

/// One person on a course, as the people page shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
    /// Only selected for staff (ADR-0048).
    pub email: Option<String>,
    pub role: CourseRole,
    pub is_owner: bool,
    pub source: MemberSource,
    pub group: Option<String>,
    pub former: bool,
}

/// A linked group with the students it brings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedGroup {
    pub link_id: Uuid,
    pub group_name: String,
    pub subgroup_name: Option<String>,
    pub student_count: i64,
    pub linked_at: DateTime<Utc>,
}

/// Who is on a course. A student gets the people without the emails, the former students and the
/// groups (ADR-0048).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoursePeople {
    pub people: Vec<Person>,
    pub groups: Vec<LinkedGroup>,
}

/// Someone who could be added to a course.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
    pub email: String,
    pub role: crate::identity::Role,
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

/// Where a roster entry comes from (ADR-0033).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberSource {
    /// A membership written on the course. It wins over a group.
    Direct,
    /// A student through a linked group or subgroup.
    Group,
}
text_enum!(MemberSource { Direct => "direct", Group => "group" });

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Membership {
    pub course_id: Uuid,
    pub user_id: Uuid,
    pub role: CourseRole,
    pub is_owner: bool,
    pub source: MemberSource,
}

/// A user's place on the roster of a course, as the view `teaching.effective_members` has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RosterEntry {
    pub role: CourseRole,
    pub source: MemberSource,
}

/// The fields of a course update that were sent. `term_id: Some(None)` clears the term.
#[derive(Debug, Clone, Default)]
pub struct CourseUpdate {
    pub title: Option<String>,
    pub term_id: Option<Option<Uuid>>,
}

/// A live link of a course to a whole group, or to one of its subgroups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CourseGroupLink {
    pub id: Uuid,
    pub course_id: Uuid,
    pub group_id: Uuid,
    pub subgroup_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// A person taken off the roster of one course although a linked group would put them on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exclusion {
    pub user_id: Uuid,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
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
    fn member_source_round_trips() {
        for source in [MemberSource::Direct, MemberSource::Group] {
            assert_eq!(source.as_str().parse::<MemberSource>(), Ok(source));
        }
        assert!("linked".parse::<MemberSource>().is_err());
    }

    #[test]
    fn course_kind_round_trips() {
        for kind in [CourseKind::Regular, CourseKind::Template] {
            assert_eq!(kind.as_str().parse::<CourseKind>(), Ok(kind));
        }
        assert!("course".parse::<CourseKind>().is_err());
    }
}
