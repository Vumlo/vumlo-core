use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use uuid::Uuid;

/// One audited action. `meta` holds ids and roles only, never personal data (AGENTS §7).
#[derive(Debug, Clone, PartialEq)]
pub struct AuditEntry {
    /// Dotted verb, e.g. `course.publish`.
    pub action: &'static str,
    pub resource_type: &'static str,
    pub resource_id: Uuid,
    pub meta: serde_json::Value,
}

/// A domain event for the outbox (ADR-0007). Payloads hold ids only: consumers fetch details
/// through the API. Each type is a public contract: add fields, never change their meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A grade was published to a student (ADR-0028).
    GradePublished {
        assignment_id: Uuid,
        student_id: Uuid,
        course_id: Uuid,
    },
    /// A student submitted an attempt (ADR-0035). Saving or discarding a draft emits nothing.
    SubmissionSubmitted {
        assignment_id: Uuid,
        student_id: Uuid,
        course_id: Uuid,
        submission_id: Uuid,
    },
    /// Someone registered with this address and must be sent the link that verifies it (ADR-0046).
    /// The token is minted by the handler, so the payload carries the user id only.
    EmailVerificationRequested { user_id: Uuid },
    /// A password reset was asked for this user; the handler mints the token and sends the link.
    PasswordResetRequested { user_id: Uuid },
}

impl Event {
    /// The dotted event type, e.g. `grade.published`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::GradePublished { .. } => "grade.published",
            Self::SubmissionSubmitted { .. } => "submission.submitted",
            Self::EmailVerificationRequested { .. } => "user.verification_requested",
            Self::PasswordResetRequested { .. } => "user.password_reset_requested",
        }
    }

    /// The version of the payload shape of [`Event::kind`].
    pub fn version(&self) -> i32 {
        match self {
            Self::GradePublished { .. }
            | Self::SubmissionSubmitted { .. }
            | Self::EmailVerificationRequested { .. }
            | Self::PasswordResetRequested { .. } => 1,
        }
    }

    pub fn payload(&self) -> serde_json::Value {
        match self {
            Self::GradePublished {
                assignment_id,
                student_id,
                course_id,
            } => serde_json::json!({
                "assignment_id": assignment_id,
                "student_id": student_id,
                "course_id": course_id,
            }),
            Self::SubmissionSubmitted {
                assignment_id,
                student_id,
                course_id,
                submission_id,
            } => serde_json::json!({
                "assignment_id": assignment_id,
                "student_id": student_id,
                "course_id": course_id,
                "submission_id": submission_id,
            }),
            Self::EmailVerificationRequested { user_id }
            | Self::PasswordResetRequested { user_id } => {
                serde_json::json!({ "user_id": user_id })
            }
        }
    }
}

/// An outbox row the worker claimed (ADR-0045).
#[derive(Debug, Clone)]
pub struct OutboxEvent {
    pub id: Uuid,
    pub kind: String,
    pub version: i32,
    pub payload: serde_json::Value,
}

/// Why a handler failed. Stored in `events.outbox.last_error`, so it must hold no secret and no
/// personal data.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct HandlerError(pub String);

pub type HandlerFuture = Pin<Box<dyn Future<Output = Result<(), HandlerError>> + Send>>;
type HandlerFn = Arc<dyn Fn(OutboxEvent) -> HandlerFuture + Send + Sync>;

/// Which function handles which event type. Built by the composition root from the handlers the
/// modules expose; a type without an entry is marked processed (ADR-0045).
#[derive(Clone, Default)]
pub struct Handlers(HashMap<&'static str, HandlerFn>);

impl Handlers {
    pub fn on(
        mut self,
        kind: &'static str,
        handler: impl Fn(OutboxEvent) -> HandlerFuture + Send + Sync + 'static,
    ) -> Self {
        self.0.insert(kind, Arc::new(handler));
        self
    }

    pub fn merge(mut self, other: Handlers) -> Self {
        self.0.extend(other.0);
        self
    }

    pub(super) fn get(&self, kind: &str) -> Option<&HandlerFn> {
        self.0.get(kind)
    }
}

/// What the administrator filters the audit log by (ADR-0053).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditFilter {
    /// A case-insensitive substring of the actor's name, the action or the label of the target.
    pub text: Option<String>,
    /// One of `accounts`, `owners`, `teachers`, `groups`, `courses`, `content`, `grades`, `terms`
    /// or `other`.
    pub kind: Option<String>,
    pub days: i32,
}

/// One row of the audit log with the names it points at (ADR-0053).
#[derive(Debug, Clone, PartialEq)]
pub struct AuditRow {
    pub id: Uuid,
    pub at: chrono::DateTime<chrono::Utc>,
    pub source: String,
    pub kind: String,
    pub action: String,
    pub actor: Option<(Uuid, String, String)>,
    pub resource_type: String,
    pub resource_id: Uuid,
    pub resource_label: Option<String>,
    pub meta: serde_json::Map<String, serde_json::Value>,
}
