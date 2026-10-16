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
}

impl Event {
    /// The dotted event type, e.g. `grade.published`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::GradePublished { .. } => "grade.published",
        }
    }

    /// The version of the payload shape of [`Event::kind`].
    pub fn version(&self) -> i32 {
        match self {
            Self::GradePublished { .. } => 1,
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
        }
    }
}
