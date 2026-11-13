use chrono::{DateTime, Utc};
use uuid::Uuid;

use access::{CourseRole, CourseStatus};
use common::text_enum;

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

/// A course after a publish, and whether the publish changed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publication {
    pub course: Course,
    /// The course was a draft; it was already published when this is false.
    pub changed: bool,
}

/// The fields of a course-info update that were sent.
#[derive(Debug, Clone, Default)]
pub struct CourseInfoUpdate {
    pub code: Option<String>,
    pub description: Option<String>,
}

/// What `teaching` knows of the header of a course (ADR-0048): the course itself and its roster.
/// The numbers that come from content, grades and attempts are added by `learning::courses`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CourseHeader {
    pub course_id: Uuid,
    pub code: String,
    pub description: String,
    pub term_name: Option<String>,
    pub owner_name: String,
    pub owner_surname: String,
    pub owner_email: String,
    pub student_count: i64,
    pub assistant_count: i64,
    pub updated_at: DateTime<Utc>,
}

/// A template of the actor's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateRow {
    pub course: Course,
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
/// groups (ADR-0048). The former students are people of other modules' data: `learning::courses`
/// adds them when `staff` is set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoursePeople {
    pub people: Vec<Person>,
    pub groups: Vec<LinkedGroup>,
    /// The asker manages the roster: they get emails, groups and the former students.
    pub staff: bool,
}

/// Someone who could be added to a course.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
    pub email: String,
    pub role: common::Role,
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
    /// The org unit the course sits in, if any (ADR-0094).
    pub unit_id: Option<Uuid>,
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

/// A course as a line of an administrator's list (ADR-0053).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CourseLineRow {
    pub course_id: Uuid,
    pub code: String,
    pub title: String,
}

/// A course of the institution with its owner, term and size (ADR-0053).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminCourseRow {
    pub course_id: Uuid,
    pub code: String,
    pub title: String,
    pub status: CourseStatus,
    pub owner_id: Uuid,
    pub owner_name: String,
    pub owner_surname: String,
    pub term_id: Option<Uuid>,
    pub term_name: Option<String>,
    pub student_count: i64,
}

/// What the list of every course is narrowed by. `q` is the text as typed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdminCourseFilter {
    pub term_id: Option<Uuid>,
    pub status: Option<CourseStatus>,
    pub q: Option<String>,
}

/// A course that uses a group (ADR-0053).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedCourseRow {
    pub course_id: Uuid,
    pub code: String,
    pub title: String,
    pub owner_name: String,
    pub owner_surname: String,
    pub linked_on: DateTime<Utc>,
}

/// What a person does in courses (ADR-0053).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserCourseFacts {
    pub owns: Vec<CourseLineRow>,
    pub teaches: Vec<CourseLineRow>,
    pub studies: i64,
}

/// A course of the caller with what its card shows of the course itself (ADR-0052).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardCourse {
    pub course: Course,
    pub code: String,
    pub description: String,
    pub term_name: Option<String>,
    pub term_ends: Option<chrono::NaiveDate>,
    pub owner_name: String,
    pub owner_surname: String,
    /// The caller's roster role; `None` for an owner who is not on the roster.
    pub role: Option<CourseRole>,
    pub student_count: i64,
    pub assistant_count: i64,
}

/// A course the caller may view and takes part in, and how (ADR-0052).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedCourse {
    pub course: CardCourse,
    /// The caller owns the course.
    pub owner: bool,
    /// The caller works in it as staff: they may manage its content.
    pub staff: bool,
    /// The caller is one of its students.
    pub student: bool,
}

/// Something a teacher told the course (ADR-0076).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announcement {
    pub id: Uuid,
    pub course_id: Uuid,
    pub author_id: Uuid,
    pub title: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests;
