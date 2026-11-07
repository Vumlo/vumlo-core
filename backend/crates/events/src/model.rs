use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chrono::{DateTime, Utc};
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// An administrator changed or reset a setting of a plugin (ADR-0072). Only that plugin is
    /// sent it. No key and no value: the plugin reads its settings again. `course_id` is `None`
    /// for an institution-wide setting.
    PluginSettingsChanged {
        install_id: Uuid,
        course_id: Option<Uuid>,
    },
    /// An assignment became visible to students for the first time (ADR-0075).
    AssignmentCreated {
        course_id: Uuid,
        assignment_id: Uuid,
    },
    /// A published assignment changed in a way students see: its configuration, points, settings
    /// or a published revision (ADR-0075).
    AssignmentUpdated {
        course_id: Uuid,
        assignment_id: Uuid,
    },
    /// The deadline of a published assignment was set, moved or cleared. An update as well, so
    /// `assignment.updated` is sent with it (ADR-0075).
    AssignmentDeadlineChanged {
        course_id: Uuid,
        assignment_id: Uuid,
    },
    /// Someone's place on the roster of a course may have changed: added, removed, a role changed,
    /// a group linked or unlinked, a member of a linked group added or removed (ADR-0075).
    EnrollmentChanged { course_id: Uuid, user_id: Uuid },
    /// A student submitted an attempt at a quiz; sent with `submission.submitted` (ADR-0075).
    QuizAttemptSubmitted {
        course_id: Uuid,
        assignment_id: Uuid,
        student_id: Uuid,
        submission_id: Uuid,
    },
    /// A teacher published an announcement in a course (ADR-0076).
    AnnouncementPublished {
        course_id: Uuid,
        announcement_id: Uuid,
    },
    /// Users were added to or removed from a group or one of its subgroups. Internal: `learning`
    /// turns it into `enrollment.changed` for the courses that use the group (ADR-0075).
    GroupMembersChanged { group_id: Uuid, user_ids: Vec<Uuid> },
}

impl Event {
    /// The dotted event type, e.g. `grade.published`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::GradePublished { .. } => "grade.published",
            Self::SubmissionSubmitted { .. } => "submission.submitted",
            Self::EmailVerificationRequested { .. } => "user.verification_requested",
            Self::PasswordResetRequested { .. } => "user.password_reset_requested",
            Self::PluginSettingsChanged { .. } => "plugin.settings.changed",
            Self::AssignmentCreated { .. } => "assignment.created",
            Self::AssignmentUpdated { .. } => "assignment.updated",
            Self::AssignmentDeadlineChanged { .. } => "assignment.deadline_changed",
            Self::EnrollmentChanged { .. } => "enrollment.changed",
            Self::QuizAttemptSubmitted { .. } => "quiz.attempt_submitted",
            Self::AnnouncementPublished { .. } => "announcement.published",
            Self::GroupMembersChanged { .. } => "group.members_changed",
        }
    }

    /// The version of the payload shape of [`Event::kind`].
    pub fn version(&self) -> i32 {
        match self {
            Self::GradePublished { .. }
            | Self::SubmissionSubmitted { .. }
            | Self::EmailVerificationRequested { .. }
            | Self::PasswordResetRequested { .. }
            | Self::PluginSettingsChanged { .. }
            | Self::AssignmentCreated { .. }
            | Self::AssignmentUpdated { .. }
            | Self::AssignmentDeadlineChanged { .. }
            | Self::EnrollmentChanged { .. }
            | Self::QuizAttemptSubmitted { .. }
            | Self::AnnouncementPublished { .. }
            | Self::GroupMembersChanged { .. } => 1,
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
            Self::PluginSettingsChanged {
                install_id,
                course_id,
            } => serde_json::json!({ "install_id": install_id, "course_id": course_id }),
            Self::AssignmentCreated {
                course_id,
                assignment_id,
            }
            | Self::AssignmentUpdated {
                course_id,
                assignment_id,
            }
            | Self::AssignmentDeadlineChanged {
                course_id,
                assignment_id,
            } => serde_json::json!({ "course_id": course_id, "assignment_id": assignment_id }),
            Self::EnrollmentChanged { course_id, user_id } => {
                serde_json::json!({ "course_id": course_id, "user_id": user_id })
            }
            Self::QuizAttemptSubmitted {
                course_id,
                assignment_id,
                student_id,
                submission_id,
            } => serde_json::json!({
                "course_id": course_id,
                "assignment_id": assignment_id,
                "student_id": student_id,
                "submission_id": submission_id,
            }),
            Self::AnnouncementPublished {
                course_id,
                announcement_id,
            } => serde_json::json!({ "course_id": course_id, "announcement_id": announcement_id }),
            Self::GroupMembersChanged { group_id, user_ids } => {
                serde_json::json!({ "group_id": group_id, "user_ids": user_ids })
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
    /// When the transaction that wrote the event began: the time the change happened.
    pub occurred_at: DateTime<Utc>,
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
