//! Request and response types of the gradebook (ADR-0028). Points are decimal strings with at
//! most two decimals (ADR-0021); responses always carry two decimals.

use chrono::{DateTime, Utc};
use garde::Validate;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{ContentStatus, optional_points_text};

/// The most students one publish request may list. `"all"` has no limit.
pub const MAX_PUBLISH_STUDENTS: usize = 1000;

/// A key that must be present, whose value may be `null`. Anything that is not a string becomes
/// `Some(value)` so that the server can reject it on the field with a fixed message.
fn required_nullable<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Null => None,
        other => Some(other),
    })
}

/// Replaces the draft cell of one student. Both keys are required: a `PUT` replaces the whole
/// cell, and `null` clears a field.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct SaveGradeRequest {
    /// A decimal string with at most two decimals, from 0 to the assignment's `max_points`, e.g.
    /// `"7.5"`; `null` clears the score. A JSON number is rejected.
    #[serde(deserialize_with = "optional_points_text")]
    #[garde(skip)]
    pub score: Option<String>,
    /// Plain text, at most 4000 characters after trimming; `null` or empty clears the feedback.
    /// Read as a JSON value so that a non-string is rejected on this field.
    #[serde(deserialize_with = "required_nullable")]
    #[schema(value_type = Option<String>)]
    #[garde(skip)]
    pub feedback: Option<Value>,
}

/// What a history row records (ADR-0028).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GradeAction {
    /// A draft was saved.
    SaveDraft,
    /// The draft was published (an empty one retracts the published value).
    Publish,
    /// A quiz was scored by the server when the student submitted it (ADR-0050); no person.
    AutoScore,
}

/// Who made a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GradeActor {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
}

/// One recorded change of a grade (ADR-0049). Data, not a sentence: the client words it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GradeHistoryEntry {
    pub id: Uuid,
    pub action: GradeAction,
    /// The score after the change, a decimal string with two decimals; `null` when none.
    pub score: Option<String>,
    /// The feedback after the change; `null` when none.
    pub feedback: Option<String>,
    /// `null` when no person made the change.
    pub actor: Option<GradeActor>,
    pub at: DateTime<Utc>,
}

/// The only value of the `students` selector that is not a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum AllStudents {
    #[serde(rename = "all")]
    All,
}

/// Which students to publish: every student whose draft differs from the published value, or a
/// listed set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum PublishStudents {
    All(AllStudents),
    Ids(Vec<Uuid>),
}

fn at_most_the_publish_limit(students: &PublishStudents, _: &()) -> garde::Result {
    match students {
        PublishStudents::Ids(ids) if ids.len() > MAX_PUBLISH_STUDENTS => Err(garde::Error::new(
            format!("must list at most {MAX_PUBLISH_STUDENTS} students"),
        )),
        _ => Ok(()),
    }
}

/// Copies draft to published. The body is required and has no default.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct PublishGradesRequest {
    /// `"all"` or a list of student ids.
    #[garde(custom(at_most_the_publish_limit))]
    pub students: PublishStudents,
}

/// The students whose published value was replaced (a retraction counts).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PublishGradesResponse {
    pub published: Vec<Uuid>,
}

/// One cell as staff see it: both slots. Never sent to a student.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GradeCell {
    /// A decimal string with exactly two decimals, or `null`.
    pub draft_score: Option<String>,
    pub draft_feedback: Option<String>,
    pub published_score: Option<String>,
    pub published_feedback: Option<String>,
    /// True when the draft differs from the published value.
    pub has_unpublished_changes: bool,
}

/// A gradebook column: one assignment of the course, in course tree order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GradebookColumn {
    pub assignment_id: Uuid,
    pub title: String,
    /// A decimal string with exactly two decimals, e.g. `"20.00"`.
    pub max_points: String,
    pub is_bonus: bool,
    /// The item's status, so staff can tell a draft column from a published one.
    pub status: ContentStatus,
}

/// The grade of one student in one column. A cell exists only where a grade row exists: the
/// student was graded in that column at least once (a draft saved, even one that was cleared).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GradebookCell {
    /// The column this cell belongs to: the `assignment_id` of an entry of `columns`. Clients
    /// build a map from it; the cells of a row are not padded to match `columns`.
    pub assignment_id: Uuid,
    #[serde(flatten)]
    pub cell: GradeCell,
}

/// One student of the gradebook with their cells.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GradebookRow {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
    /// Not on the roster now (left the group, disabled, promoted, excluded) but has grades in the
    /// course. A former student's grades are kept and read-only: a write answers `409` and
    /// "publish all" skips them (ADR-0037).
    #[serde(default)]
    pub former: bool,
    /// Only the cells that exist, in column order, each carrying its `assignment_id`. A column
    /// with no cell here means this student has no grade there: absence is "no grade", not an
    /// error and not a cell with empty values.
    pub cells: Vec<GradebookCell>,
}

/// A page of the staff gradebook, paged by student (ADR-0018). `columns` is the same on every
/// page; each row's `cells` refer to a column by `assignment_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GradebookPage {
    pub columns: Vec<GradebookColumn>,
    pub items: Vec<GradebookRow>,
    /// Pass as `cursor` to get the next page; `null` on the last page.
    pub next_cursor: Option<String>,
}

/// A published grade as its student sees it. It has no draft field by construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct MyGrade {
    pub assignment_id: Uuid,
    pub title: String,
    /// A decimal string with exactly two decimals, e.g. `"20.00"`.
    pub max_points: String,
    pub is_bonus: bool,
    /// A decimal string with exactly two decimals, or `null` if only feedback was published.
    pub score: Option<String>,
    pub feedback: Option<String>,
    pub published_at: DateTime<Utc>,
}

/// The published grades of the caller in one course, in course tree order. Not paged: a course
/// is bounded (ADR-0020).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct MyGrades {
    pub items: Vec<MyGrade>,
}

#[cfg(test)]
mod tests;
