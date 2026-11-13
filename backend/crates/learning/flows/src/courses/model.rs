use chrono::{DateTime, Utc};
use sqlx::types::Decimal;
use uuid::Uuid;

use crate::content::NextDeadline;
use crate::gradebook::Progress;
use teaching::{CardCourse, Course};

/// One entry of a staff card's to-do list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Todo {
    ToGrade { count: i64 },
    DraftGrades { count: i64, auto_scored: bool },
    UnpublishedItems { count: i64, titles: Vec<String> },
    NextDeadline(NextDeadline),
}

/// The card of one course, whichever side of it the caller is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CourseCardData {
    pub course: CardCourse,
    pub owner: bool,
    pub progress: Option<Progress>,
    pub next: Option<NextDeadline>,
    pub todo: Vec<Todo>,
}

/// The header of a course (ADR-0048): the course and its roster from `teaching`, what the content
/// adds up to, and whether grades or attempts keep it from being deleted.
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
    /// People off the roster who still have a value or an attempt in the course (ADR-0037).
    pub former_count: i64,
    pub assistant_count: i64,
    pub updated_at: DateTime<Utc>,
    pub content_updated_at: Option<DateTime<Utc>>,
    pub total_points: Decimal,
    pub graded_items: i64,
    pub has_grades: bool,
}

/// A template and what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub course: Course,
    pub sections: i64,
    pub lectures: i64,
    pub assignments: i64,
    pub updated_at: DateTime<Utc>,
}
