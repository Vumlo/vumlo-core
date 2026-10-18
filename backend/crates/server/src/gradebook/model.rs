use chrono::{DateTime, Utc};
use sqlx::types::Decimal;
use uuid::Uuid;

use crate::points;
use db::text_enum;

const FEEDBACK_MAX_LEN: usize = 4000;

/// One row of `gradebook.grade_history`: the values after a save or a publish (ADR-0028).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: Uuid,
    /// `save_draft`, `publish` or `auto_score` (a quiz scored by the server, no person).
    pub action: String,
    pub score: Option<Decimal>,
    pub feedback: Option<String>,
    pub actor: Option<HistoryActor>,
    pub at: DateTime<Utc>,
}

/// A student's latest attempt that has no draft score (ADR-0052).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToGradeRow {
    pub student_id: Uuid,
    pub student_name: String,
    pub student_surname: String,
    pub assignment_id: Uuid,
    pub assignment_title: String,
    pub course_id: Uuid,
    pub submitted_at: DateTime<Utc>,
    pub attempt: i32,
    pub max_attempts: Option<i32>,
}

/// An assignment with draft grades that are not published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnpublishedRow {
    pub assignment_id: Uuid,
    pub assignment_title: String,
    pub course_id: Uuid,
    pub draft_count: i64,
    pub auto_scored: bool,
}

/// An assignment due from now on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadlineRow {
    pub assignment_id: Uuid,
    pub title: String,
    pub course_id: Uuid,
    pub due_at: DateTime<Utc>,
    pub submitted: i64,
    pub total: i64,
    pub not_open: bool,
}

/// An item whose content students cannot read yet; `edits` is false for a draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnpublishedContentRow {
    pub item_id: Uuid,
    pub title: String,
    pub course_id: Uuid,
    pub edits: bool,
}

/// What a teacher has to do across their courses (ADR-0052).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeachingOverview {
    pub courses: Vec<crate::teaching::CardCourse>,
    pub to_grade: Vec<ToGradeRow>,
    pub unpublished: Vec<UnpublishedRow>,
    pub deadlines: Vec<DeadlineRow>,
    pub unpublished_content: Vec<UnpublishedContentRow>,
}

/// The published slot of one cell: what the student may read (ADR-0028).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedGrade {
    pub score: Option<Decimal>,
    pub feedback: Option<String>,
    pub published_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryActor {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
}

/// A value that failed validation. Each variant maps to one field of the API.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    /// Not a decimal string with at most two decimals from 0 to the assignment's `max_points`.
    #[error("must be a decimal string from 0 to {max:.2} with at most two decimals")]
    Score { max: Decimal },
    #[error("must be at most {FEEDBACK_MAX_LEN} characters of plain text")]
    Feedback,
}

/// One rejected field of a grade write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldProblem {
    pub field: &'static str,
    pub message: String,
}

/// A valid score: `0 <= score <= max` with at most two decimals. Zero is a score.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Score(Decimal);

impl Score {
    /// `max` is the `max_points` of the assignment, read from its locked row.
    pub fn parse(raw: &str, max: Decimal) -> Result<Self, ModelError> {
        points::parse_decimal(raw)
            .filter(|value| *value <= max)
            .map(Self)
            .ok_or(ModelError::Score { max })
    }

    pub fn value(self) -> Decimal {
        self.0
    }
}

/// Plain text, trimmed, at most 4000 characters. Newlines and tabs are allowed, other control
/// characters are not. Text that is empty after trimming is no feedback at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feedback(String);

impl Feedback {
    pub fn parse(raw: &str) -> Result<Option<Self>, ModelError> {
        let text = raw.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let plain = !text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t' | '\r'));
        if plain && text.chars().count() <= FEEDBACK_MAX_LEN {
            Ok(Some(Self(text.to_owned())))
        } else {
            Err(ModelError::Feedback)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What the client sent for `feedback`. A value that is not text is judged after the actor was
/// authorized, like every other value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedbackInput {
    Text(String),
    NotText,
}

/// The raw draft cell a client sends: `None` clears a field. Nothing in it is trusted before
/// the service has authorized the actor and read `max_points`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GradeInput {
    pub score: Option<String>,
    pub feedback: Option<FeedbackInput>,
}

/// Which students a publish applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Students {
    All,
    Listed(Vec<Uuid>),
}

/// A grade cell as staff see it: both slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub draft_score: Option<Decimal>,
    pub draft_feedback: Option<String>,
    pub published_score: Option<Decimal>,
    pub published_feedback: Option<String>,
    pub has_unpublished_changes: bool,
}

/// The status of the item behind a column, so staff can tell a draft column from a published one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnStatus {
    Draft,
    Published,
}
text_enum!(ColumnStatus { Draft => "draft", Published => "published" });

/// One assignment of the course, in course tree order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub assignment_id: Uuid,
    pub title: String,
    pub max_points: Decimal,
    pub is_bonus: bool,
    pub status: ColumnStatus,
}

/// One student of the gradebook. `cells` holds only the assignments the student has a grade row
/// for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GradebookRow {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
    /// Not on the roster now (left, disabled, promoted, excluded); the grades are kept and read-only
    /// (ADR-0037).
    pub former: bool,
    pub cells: Vec<(Uuid, Cell)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GradebookPage {
    pub columns: Vec<Column>,
    pub rows: Vec<GradebookRow>,
    /// The cursor for the next page, `None` on the last one.
    pub next: Option<Uuid>,
}

/// A published grade as its student sees it. There is no draft field in this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MyGrade {
    pub assignment_id: Uuid,
    pub title: String,
    pub max_points: Decimal,
    pub is_bonus: bool,
    pub score: Option<Decimal>,
    pub feedback: Option<String>,
    pub published_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests;
