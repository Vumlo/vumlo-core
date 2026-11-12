//! The write tools for grading and deadlines (ADR-0070). None of them runs from the agent loop: the
//! loop only [`Write::parse`]s and [`Write::describe`]s one into a proposal, and the service
//! function runs when the user confirms. Each one is a call of the core's own service function as the
//! user, so authorization, validation, the audit row and the events are the core's.

use std::collections::BTreeMap;

use access::Actor;
use chrono::{DateTime, Utc};
use common::{ApiError, PageParams};
use learning::content;
use learning::gradebook::{self, FeedbackInput, GradeInput, Students};
use learning::items;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::llm::ToolSpec;

/// What the gradebook is asked for when it looks a student up: a course has far fewer.
const ROSTER_PAGE: u32 = 200;

/// A change to the gradebook or a deadline, as the model asked for it and the user will see it.
/// It is stored as JSON in the proposal and parsed again at confirm.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case", deny_unknown_fields)]
pub enum Write {
    /// Replaces the whole draft of one student: an empty field is cleared.
    SetGrade {
        assignment_id: Uuid,
        student_id: Uuid,
        #[serde(deserialize_with = "text_or_number")]
        score: Option<String>,
        #[serde(deserialize_with = "text_or_number")]
        feedback: Option<String>,
    },
    PublishGrades {
        assignment_id: Uuid,
    },
    SetDeadline {
        assignment_id: Uuid,
        due_at: Option<DateTime<Utc>>,
    },
}

/// A model writes `8` as often as `"8"`.
pub(super) fn text_or_number<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(s)),
        Value::Number(n) => Ok(Some(n.to_string())),
        _ => Err(serde::de::Error::custom("expected text, a number or null")),
    }
}

pub const NAMES: [&str; 3] = ["set_grade", "publish_grades", "set_deadline"];

pub fn specs() -> Vec<ToolSpec> {
    let uuid = |what: &str| json!({"type": "string", "description": what});
    vec![
        ToolSpec::new(
            "set_grade",
            "Prepares saving a DRAFT grade for one student on one assignment; students do not see it \
             until grades are published. It replaces the whole draft: send both score and feedback, \
             null for one left empty (read the current ones with course_gradebook first when keeping \
             one). The user must confirm before anything is saved.",
            json!({
                "type": "object",
                "properties": {
                    "assignment_id": uuid("An assignment id from course_gradebook"),
                    "student_id": uuid("A student id from course_gradebook or teaching_overview"),
                    "score": {"type": ["string", "number", "null"], "description": "Points, at most the assignment's max_points"},
                    "feedback": {"type": ["string", "null"]}
                },
                "required": ["assignment_id", "student_id", "score", "feedback"]
            }),
        ),
        ToolSpec::new(
            "publish_grades",
            "Prepares publishing the draft grades of one assignment, so its students see them. The \
             user must confirm before anything is published.",
            json!({
                "type": "object",
                "properties": {"assignment_id": uuid("An assignment id from course_gradebook")},
                "required": ["assignment_id"]
            }),
        ),
        ToolSpec::new(
            "set_deadline",
            "Prepares setting, moving or removing the deadline of one assignment. The user must \
             confirm before anything changes.",
            json!({
                "type": "object",
                "properties": {
                    "assignment_id": uuid("An assignment id from course_gradebook or teaching_overview"),
                    "due_at": {"type": ["string", "null"], "description": "The new deadline as an RFC 3339 time with its zone, such as 2026-12-01T23:59:00+02:00; null removes the deadline"}
                },
                "required": ["assignment_id", "due_at"]
            }),
        ),
    ]
}

impl Write {
    /// Reads what the model sent. Everything is untrusted: the fields that would change meaning if
    /// left out (`null` clears) must be there.
    pub fn parse(name: &str, arguments: &str) -> Result<Self, String> {
        let bad = || "the arguments do not fit this tool".to_owned();
        let raw = if arguments.trim().is_empty() {
            "{}"
        } else {
            arguments
        };
        let mut value: Value = serde_json::from_str(raw).map_err(|_| bad())?;
        let object = value.as_object_mut().ok_or_else(bad)?;
        let required: &[&str] = match name {
            "set_grade" => &["score", "feedback"],
            "set_deadline" => &["due_at"],
            _ => &[],
        };
        for key in required {
            if !object.contains_key(*key) {
                return Err(format!("{key} is required: send null to leave it empty"));
            }
        }
        object.insert("tool".to_owned(), Value::String(name.to_owned()));
        serde_json::from_value(value).map_err(|e| format!("the arguments do not fit {name}: {e}"))
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::SetGrade { .. } => NAMES[0],
            Self::PublishGrades { .. } => NAMES[1],
            Self::SetDeadline { .. } => NAMES[2],
        }
    }

    fn assignment_id(&self) -> Uuid {
        match self {
            Self::SetGrade { assignment_id, .. }
            | Self::PublishGrades { assignment_id }
            | Self::SetDeadline { assignment_id, .. } => *assignment_id,
        }
    }

    /// What the card will say, read from the service as the user: titles and names the user may
    /// see, never words of the model. An error is for the model: the target is not there for them.
    pub async fn describe(
        &self,
        conn: &mut PgConnection,
        actor: &Actor,
    ) -> Result<BTreeMap<&'static str, String>, String> {
        let item = items::get(conn, actor, self.assignment_id())
            .await
            .map_err(|e| explain(&ApiError::from(e)))?;
        let Some(assignment) = &item.assignment else {
            return Err("that item is not an assignment".to_owned());
        };
        let mut facts = BTreeMap::from([("assignment", item.title.clone())]);
        match self {
            Self::SetGrade {
                student_id,
                score,
                feedback,
                ..
            } => {
                let student = student_name(conn, actor, item.course_id, *student_id).await?;
                facts.insert(
                    "student",
                    student.ok_or("that student is not in the course")?,
                );
                facts.insert("score", score.clone().unwrap_or_default());
                facts.insert("max_points", assignment.max_points.to_string());
                facts.insert("feedback", feedback.clone().unwrap_or_default());
            }
            Self::PublishGrades { .. } => {}
            Self::SetDeadline { due_at, .. } => {
                facts.insert("due_at", due_at.map(|d| d.to_rfc3339()).unwrap_or_default());
            }
        }
        Ok(facts)
    }

    /// Makes the change. The caller owns the transaction.
    pub async fn run(&self, conn: &mut PgConnection, actor: &Actor) -> Result<(), ApiError> {
        match self {
            Self::SetGrade {
                assignment_id,
                student_id,
                score,
                feedback,
            } => {
                let input = GradeInput {
                    score: score.clone(),
                    feedback: feedback.clone().map(FeedbackInput::Text),
                };
                gradebook::save_grade(conn, actor, *assignment_id, *student_id, input)
                    .await
                    .map_err(ApiError::from)?;
            }
            Self::PublishGrades { assignment_id } => {
                gradebook::publish_grades(conn, actor, *assignment_id, Students::All)
                    .await
                    .map_err(ApiError::from)?;
            }
            Self::SetDeadline {
                assignment_id,
                due_at,
            } => {
                content::set_due_at(conn, actor, *assignment_id, *due_at)
                    .await
                    .map_err(ApiError::from)?;
            }
        }
        Ok(())
    }
}

/// "Name Surname" of a current student of the course, from the gradebook the user may read.
async fn student_name(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    student_id: Uuid,
) -> Result<Option<String>, String> {
    let mut after = None;
    loop {
        let page = gradebook::gradebook(
            conn,
            actor,
            false,
            course_id,
            PageParams {
                after,
                limit: ROSTER_PAGE,
            },
        )
        .await
        .map_err(|e| explain(&ApiError::from(e)))?;
        if let Some(row) = page.rows.iter().find(|r| r.user_id == student_id) {
            return Ok(Some(format!("{} {}", row.name, row.surname)));
        }
        match page.next {
            Some(next) => after = Some(next),
            None => return Ok(None),
        }
    }
}

/// Why a change was refused, in words for the model (and, at confirm, for the user).
pub fn explain(error: &ApiError) -> String {
    match error {
        ApiError::NotFound => "not found, or the user may not see it".to_owned(),
        ApiError::Forbidden(m) | ApiError::Conflict(m) | ApiError::Gone(m) => (*m).to_owned(),
        ApiError::Unprocessable { detail, errors } if errors.is_empty() => detail.clone(),
        ApiError::Unprocessable { errors, .. } => errors
            .iter()
            .map(|e| format!("{}: {}", e.field, e.message))
            .collect::<Vec<_>>()
            .join("; "),
        _ => "the change could not be made right now".to_owned(),
    }
}

#[cfg(test)]
mod tests;
