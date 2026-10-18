//! The read models of the home screens and search (ADR-0052). They carry numbers and kinds, never
//! sentences: the client words them.

use chrono::{DateTime, Utc};
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::{Course, WorkStatus};

/// The caller's place in a course.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CardRole {
    Owner,
    Teacher,
    Assistant,
    Student,
}

/// A student's numbers over the assignments of a course that take submissions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CardProgress {
    pub submitted: i64,
    pub total: i64,
    pub graded: i64,
}

/// The next piece of work a student has to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CardNext {
    pub assignment_id: Uuid,
    pub title: String,
    pub due_at: DateTime<Utc>,
}

/// One thing a teacher has to do in a course. Entries with a count of 0 are left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CardTodo {
    /// Submitted work with no draft score.
    ToGrade { count: i64 },
    /// Draft grades that are not published; `auto_scored` when they come from quizzes.
    DraftGrades { count: i64, auto_scored: bool },
    /// Draft items inside published modules; at most three titles.
    UnpublishedItems { count: i64, titles: Vec<String> },
    /// The next deadline of the course.
    NextDeadline {
        assignment_id: Uuid,
        title: String,
        due_at: DateTime<Utc>,
    },
}

/// One course of the caller on the course list and the home page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CourseCard {
    pub course: Course,
    pub code: String,
    pub term_name: Option<String>,
    /// The term's last day is before today in the institution's time zone.
    pub term_ended: bool,
    pub owner_name: String,
    pub owner_surname: String,
    pub my_role: CardRole,
    pub student_count: i64,
    pub assistant_count: i64,
    /// Students only.
    pub progress: Option<CardProgress>,
    /// Students only.
    pub next: Option<CardNext>,
    /// Staff only.
    pub todo: Vec<CardTodo>,
}

/// The course cards of the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CourseCards {
    pub items: Vec<CourseCard>,
}

/// A course a student studies in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct WorkCourse {
    pub id: Uuid,
    pub code: String,
    pub title: String,
    pub term_name: Option<String>,
    pub owner_name: String,
    pub owner_surname: String,
}

/// One assignment and the caller's own standing on it. Only published values appear.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct WorkItem {
    pub assignment_id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    #[serde(rename = "type")]
    pub assignment_type: String,
    pub due_at: Option<DateTime<Utc>>,
    /// The day an in-class assignment takes place, `YYYY-MM-DD`.
    pub class_on: Option<String>,
    pub max_points: String,
    pub status: WorkStatus,
    /// The published score, a decimal string with two decimals.
    pub score: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub draft_saved_at: Option<DateTime<Utc>>,
}

/// Every assignment of the caller's courses with the caller's own progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct MyWork {
    pub courses: Vec<WorkCourse>,
    pub items: Vec<WorkItem>,
}

/// A course, as a line of the teacher's overview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CourseRef {
    pub course_id: Uuid,
    pub course_code: String,
    pub course_title: String,
}

/// A student's latest attempt that has no draft score yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ToGrade {
    pub student_id: Uuid,
    pub student_name: String,
    pub student_surname: String,
    pub assignment_id: Uuid,
    pub assignment_title: String,
    #[serde(flatten)]
    pub course: CourseRef,
    pub submitted_at: DateTime<Utc>,
    pub attempt: i32,
    pub max_attempts: Option<i32>,
}

/// An assignment whose draft grades are not published.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UnpublishedGrades {
    pub assignment_id: Uuid,
    pub assignment_title: String,
    #[serde(flatten)]
    pub course: CourseRef,
    pub draft_count: i64,
    /// The assignment is a quiz, so the drafts were scored by the server.
    pub auto_scored: bool,
}

/// An assignment due from now on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Deadline {
    pub assignment_id: Uuid,
    pub title: String,
    #[serde(flatten)]
    pub course: CourseRef,
    pub due_at: DateTime<Utc>,
    pub submitted: i64,
    pub total: i64,
    /// The assignment is a draft: students do not see it yet.
    pub not_open: bool,
}

/// Why an item is on the list of unpublished content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnpublishedReason {
    /// The item is a draft inside a published module.
    Draft,
    /// The item is published and has changes students do not see.
    Edits,
}

/// An item with content students cannot read yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UnpublishedContent {
    pub item_id: Uuid,
    pub title: String,
    #[serde(flatten)]
    pub course: CourseRef,
    pub reason: UnpublishedReason,
}

/// What a teacher has to do across all their courses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TeachingOverview {
    pub to_grade: Vec<ToGrade>,
    pub unpublished: Vec<UnpublishedGrades>,
    pub deadlines: Vec<Deadline>,
    pub unpublished_content: Vec<UnpublishedContent>,
}

/// What a search hit is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HitKind {
    Course,
    Lecture,
    Assignment,
}

/// Whether a hit is visible to students.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HitStatus {
    Draft,
    Published,
}

/// A course, or a lecture or an assignment in one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SearchHit {
    pub kind: HitKind,
    pub id: Uuid,
    pub title: String,
    #[serde(flatten)]
    pub course: CourseRef,
    /// A short piece of text around the match; `null` for a course or a match in the title.
    pub snippet: Option<String>,
    /// For an assignment, when it is due.
    pub due_at: Option<DateTime<Utc>>,
    pub status: HitStatus,
}

/// The hits, best first. There is no next page: the list is cut at 50.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SearchResults {
    pub items: Vec<SearchHit>,
    pub next_cursor: Option<String>,
}

/// The query of a search.
#[derive(Debug, Clone, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SearchQuery {
    /// What to look for, 2 to 100 characters.
    #[garde(length(chars, min = 2, max = 100))]
    pub q: String,
}
