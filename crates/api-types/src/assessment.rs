//! Request and response types of the submissions (ADR-0011, ADR-0035, ADR-0050, ADR-0051). A
//! submission's `content` is a block document (ADR-0025) of the `student` profile for a `text`
//! assignment and `{"answers": {...}}` for a `quiz` or a `form`.

use chrono::{DateTime, Utc};
use garde::Validate;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::ToSchema;
use uuid::Uuid;

/// What happens to a draft at `due_at` (ADR-0011). `auto_submit` is stored and validated, and
/// takes effect with the background worker (ADR-0035).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OnDeadline {
    KeepDraft,
    AutoSubmit,
}

/// Replaces the draft of the caller. The content is read as a JSON value so that a rejection can
/// name the path inside it, for example `content.blocks[2].props.level` or
/// `content.answers.q1`. Its shape is that of the assignment's type.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct SaveDraftRequest {
    #[schema(value_type = Object)]
    #[garde(skip)]
    pub content: Value,
}

/// The caller's draft: the one unsubmitted document of a student for an assignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SubmissionDraft {
    #[schema(value_type = Object)]
    pub content: Map<String, Value>,
    pub updated_at: DateTime<Utc>,
}

/// One submitted attempt. Attempts are numbered from 1 per student and assignment and never change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Attempt {
    pub id: Uuid,
    pub attempt: i32,
    #[schema(value_type = Object)]
    pub content: Map<String, Value>,
    pub submitted_at: DateTime<Utc>,
    /// True when it was submitted after `due_at`, inside the late window.
    pub is_late: bool,
    /// Staff only, for a `quiz`: the score of each question that was scored by the server, as
    /// decimal strings (ADR-0050). A `long_text` question has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub auto_scores: Option<Map<String, Value>>,
}

/// The caller's own work on an assignment: the draft, if there is one, and the attempts, newest
/// first. A client prefills a new draft from `attempts[0]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct MySubmission {
    pub draft: Option<SubmissionDraft>,
    pub attempts: Vec<Attempt>,
    /// Pass as `cursor` to get the older attempts; `null` on the last page.
    pub next_cursor: Option<String>,
}

/// A page of the attempts of one student, newest first (staff).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AttemptsPage {
    pub items: Vec<Attempt>,
    pub next_cursor: Option<String>,
}

/// What staff see of a student in the list: `submitted` when the student has an attempt, otherwise
/// `not_submitted`. A draft never shows (ADR-0011).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionStatus {
    Submitted,
    NotSubmitted,
}

/// The latest submitted attempt of a student, without its content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct LatestAttempt {
    pub id: Uuid,
    pub attempt: i32,
    pub submitted_at: DateTime<Utc>,
    pub is_late: bool,
}

/// One student of the course with their latest submitted attempt, if any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SubmissionRow {
    pub student_id: Uuid,
    pub name: String,
    pub surname: String,
    /// Not on the roster now but has a submitted attempt here (ADR-0037). The attempts stay
    /// readable; a former student cannot be graded.
    #[serde(default)]
    pub former: bool,
    pub status: SubmissionStatus,
    pub latest: Option<LatestAttempt>,
}

/// A page of the students of a course with their latest attempts, by surname, name and id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SubmissionsPage {
    pub items: Vec<SubmissionRow>,
    pub next_cursor: Option<String>,
}

/// The result of a quiz for the student who took it (ADR-0050 point 8). Only once the grade is
/// published.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct QuizResult {
    /// The published score, a decimal string with two decimals.
    pub score: String,
    pub max_points: String,
    pub published_at: DateTime<Utc>,
    /// The published feedback; `null` when none.
    pub feedback: Option<String>,
    /// The attempt the answers belong to: the latest.
    pub attempt: i32,
    pub questions: Vec<QuizResultQuestion>,
}

/// One question of a quiz result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct QuizResultQuestion {
    pub id: String,
    pub points: String,
    /// What the server scored, a decimal string; `null` for a question graded by hand.
    pub got: Option<String>,
    /// What the student answered, as stored; `null` when unanswered.
    #[schema(value_type = Option<Object>)]
    pub answer: Option<Value>,
    /// The correct answer: option ids, the number, or the accepted texts. Present only when the quiz
    /// shows its answers.
    #[schema(value_type = Option<Object>)]
    pub correct: Option<Value>,
    /// The explanation, a block document. Present only when the quiz shows its answers.
    #[schema(value_type = Option<Object>)]
    pub explanation: Option<Value>,
}

/// A student of the course, named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct StudentName {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
}

/// The answers of one student to a form: their latest attempt (ADR-0051 point 7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FormResponseRow {
    pub student_id: Uuid,
    pub name: String,
    pub surname: String,
    pub attempt: i32,
    pub submitted_at: DateTime<Utc>,
    pub is_late: bool,
    /// Field id to the answer, as stored.
    #[schema(value_type = Object)]
    pub answers: Map<String, Value>,
}

/// All the responses to a form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FormResponses {
    /// The current students of the course.
    pub total_students: i64,
    /// Current students with no attempt.
    pub missing: Vec<StudentName>,
    pub rows: Vec<FormResponseRow>,
}
