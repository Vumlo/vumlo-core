//! Request and response types of the submissions (ADR-0011, ADR-0035). A submission's `content`
//! is a block document (ADR-0025) of the `student` profile.

use chrono::{DateTime, Utc};
use garde::Validate;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::Document;

/// What happens to a draft at `due_at` (ADR-0011). `auto_submit` is stored and validated, and
/// takes effect with the background worker (ADR-0035).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OnDeadline {
    KeepDraft,
    AutoSubmit,
}

/// Replaces the draft of the caller. The document is read as a JSON value so that a rejection can
/// name the path inside it, for example `content.blocks[2].props.level`.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct SaveDraftRequest {
    #[schema(value_type = Document)]
    #[garde(skip)]
    pub content: Value,
}

/// The caller's draft: the one unsubmitted document of a student for an assignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SubmissionDraft {
    #[schema(value_type = Document)]
    pub content: Map<String, Value>,
    pub updated_at: DateTime<Utc>,
}

/// One submitted attempt. Attempts are numbered from 1 per student and assignment and never change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Attempt {
    pub id: Uuid,
    pub attempt: i32,
    #[schema(value_type = Document)]
    pub content: Map<String, Value>,
    pub submitted_at: DateTime<Utc>,
    /// True when it was submitted after `due_at`, inside the late window.
    pub is_late: bool,
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
