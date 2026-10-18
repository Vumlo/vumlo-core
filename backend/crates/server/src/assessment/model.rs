use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sqlx::types::Decimal;
use uuid::Uuid;

use crate::content::{Holders, ResultQuestion, SubmissionSettings};
use crate::pagination::Page;

/// The unsubmitted document of a student for an assignment (ADR-0011). Only its owner reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub content: Map<String, Value>,
    pub updated_at: DateTime<Utc>,
}

/// A submitted attempt. It never changes (ADR-0011, ADR-0035).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    pub id: Uuid,
    pub attempt: i32,
    pub content: Map<String, Value>,
    pub submitted_at: DateTime<Utc>,
    pub is_late: bool,
    /// The per-question scores of a quiz (ADR-0050). Read only on the staff path: a student's
    /// reads never fill it.
    pub auto_scores: Option<Map<String, Value>>,
}

/// A current student, named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudentName {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
}

/// The latest attempt of a student with its content (ADR-0051).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestContent {
    pub student: StudentName,
    pub attempt: i32,
    pub submitted_at: DateTime<Utc>,
    pub is_late: bool,
    pub content: Map<String, Value>,
}

/// The responses to a form (ADR-0051 point 7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Responses {
    pub total_students: i64,
    pub missing: Vec<StudentName>,
    pub rows: Vec<LatestContent>,
}

/// What a form's config shows besides what was configured (ADR-0051 point 5).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormFacts {
    /// Ids of the fields that have an answer in a latest attempt.
    pub answered: HashSet<String>,
    pub holders: Holders,
}

/// A quiz result as the service builds it (ADR-0050 point 8).
#[derive(Debug, Clone, PartialEq)]
pub struct QuizResult {
    pub score: Decimal,
    pub max_points: Decimal,
    pub published_at: DateTime<Utc>,
    pub feedback: Option<String>,
    pub attempt: i32,
    pub questions: Vec<ResultQuestion>,
}

/// A student's own work: the draft, if any, and a page of attempts, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MySubmission {
    pub draft: Option<Draft>,
    pub attempts: Page<Attempt>,
}

/// The latest submitted attempt of a student, without its content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestAttempt {
    pub id: Uuid,
    pub attempt: i32,
    pub submitted_at: DateTime<Utc>,
    pub is_late: bool,
}

/// A student of the course and their latest attempt, if there is one (staff list). A former
/// student is one who left the roster but has an attempt (ADR-0037).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudentSubmission {
    pub student_id: Uuid,
    pub name: String,
    pub surname: String,
    pub former: bool,
    pub latest: Option<LatestAttempt>,
}

/// Whether a save or a submit is taken at some moment (ADR-0035 point 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    /// Taken; `late` when it is after `due_at`.
    Open {
        late: bool,
    },
    Closed,
}

impl Window {
    /// The window at `now`. It closes right after `due_at` when late work is not accepted, and
    /// right after `late_until` (never, when unset) when it is; a moment equal to either bound is
    /// still inside. Without a deadline it never closes. The settings come from the locked
    /// assignment row, the time from the caller.
    pub fn at(settings: &SubmissionSettings, now: DateTime<Utc>) -> Self {
        let Some(due_at) = settings.due_at else {
            return Self::Open { late: false };
        };
        if now <= due_at {
            return Self::Open { late: false };
        }
        match (settings.accept_late, settings.late_until) {
            (Some(true), None) => Self::Open { late: true },
            (Some(true), Some(until)) if now <= until => Self::Open { late: true },
            _ => Self::Closed,
        }
    }
}

#[cfg(test)]
mod tests;
