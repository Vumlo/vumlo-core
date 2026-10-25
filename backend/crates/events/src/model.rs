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

/// Which functions handle which event type. Built by the composition root from the handlers the
/// modules expose; a type without an entry is marked processed (ADR-0045). A type may have several
/// handlers, in the order they were added (ADR-0066): all must succeed, and when one fails the
/// event is delivered again to all of them, so every handler tolerates a repeat.
#[derive(Clone, Default)]
pub struct Handlers(HashMap<&'static str, Vec<HandlerFn>>);

impl Handlers {
    pub fn on(
        mut self,
        kind: &'static str,
        handler: impl Fn(OutboxEvent) -> HandlerFuture + Send + Sync + 'static,
    ) -> Self {
        self.0.entry(kind).or_default().push(Arc::new(handler));
        self
    }

    pub fn merge(mut self, other: Handlers) -> Self {
        for (kind, mut handlers) in other.0 {
            self.0.entry(kind).or_default().append(&mut handlers);
        }
        self
    }

    pub(super) fn get(&self, kind: &str) -> &[HandlerFn] {
        self.0.get(kind).map_or(&[], Vec::as_slice)
    }
}
