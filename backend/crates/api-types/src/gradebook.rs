//! Request and response types of the gradebook (ADR-0028). Points are decimal strings with at
//! most two decimals (ADR-0021); responses always carry two decimals.

use chrono::{DateTime, Utc};
use garde::Validate;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{ContentStatus, optional_points_text, points_text};

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
    /// The `key` of the group the assignment is counted in (ADR-0089); `null` when it is in none
    /// ("Not sorted"). Absent from a twelve-point installation, which has no groups.
    #[serde(default)]
    pub group: Option<Uuid>,
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
    /// What this student's marks add up to (ADR-0089); absent on the twelve-point scale.
    #[serde(default)]
    pub totals: Option<GradebookTotals>,
}

/// A page of the staff gradebook, paged by student (ADR-0018). `columns` is the same on every
/// page; each row's `cells` refer to a column by `assignment_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GradebookPage {
    pub columns: Vec<GradebookColumn>,
    pub items: Vec<GradebookRow>,
    /// The settings, the groups and the setup status, the same on every page (ADR-0089); absent
    /// on the twelve-point scale.
    #[serde(default)]
    pub layout: Option<GradebookLayoutView>,
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

fn yes() -> bool {
    true
}

/// Where a course's groups come from (ADR-0089).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GradebookLayout {
    /// A group stands for a top-level module of the course.
    Course,
    /// The teacher's own groups.
    Custom,
}

/// What a course counts toward (ADR-0089).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Validate, ToSchema)]
pub struct GradebookSettings {
    /// The points that make a full result, a decimal string above 0, at most 9999; usually `"100"`.
    #[serde(deserialize_with = "points_text")]
    #[garde(skip)]
    pub target: String,
    /// The mark that passes, a decimal string from 0 to the target; `null` or absent = none.
    #[serde(default, deserialize_with = "optional_points_text")]
    #[garde(skip)]
    pub pass_mark: Option<String>,
    #[garde(skip)]
    pub layout: GradebookLayout,
}

/// A group of assignments and its rules (ADR-0089).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GradebookGroup {
    /// The top-level module's id in the `course` layout, the group's own id in the `custom` one.
    pub key: Uuid,
    pub title: String,
    /// The most the group adds to the total, a decimal string; `null` = no cap.
    pub cap: Option<String>,
    /// Only the best K marks count; `null` = all of them.
    pub best_k: Option<u32>,
    /// False = practice: graded and shown, not part of the total.
    pub counts: bool,
    /// True = extra: adds to the total, not to what a student can still reach.
    pub bonus: bool,
}

/// What is left to do before the gradebook is in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GradebookSetup {
    /// Assignments in no group, in the `custom` layout.
    pub unsorted: u32,
    /// Keys of the groups that have no assignment.
    pub empty_groups: Vec<Uuid>,
    /// What a student can reach, bonus left out; a decimal string with two decimals.
    pub possible: String,
    pub target: String,
    /// What a student can reach is at least the target.
    pub reaches_target: bool,
}

/// How a course turns points into a result, the same for every student (ADR-0089).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GradebookLayoutView {
    pub settings: GradebookSettings,
    /// The groups in the order they are shown.
    pub groups: Vec<GradebookGroup>,
    pub possible: String,
    pub setup: GradebookSetup,
}

/// What one group of one student adds up to, for the staff table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GradebookGroupResult {
    /// The group's `key`; `null` for the assignments that are in no group.
    pub key: Option<Uuid>,
    pub total: String,
    /// The cap cut the sum.
    pub capped: bool,
    /// Assignments left out by the "best K" rule.
    pub dropped: Vec<Uuid>,
}

/// What one student's marks add up to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GradebookOutcome {
    /// The final total, never more than the target; a decimal string with two decimals.
    pub total: String,
    /// Every point earned, before the target cuts it (116 when the target is 100); the same as
    /// `total` when it is not more than the target.
    pub earned: String,
    /// `null` when the course has no pass mark.
    pub passed: Option<bool>,
    pub groups: Vec<GradebookGroupResult>,
}

/// A student's total from the published slots, and from the drafts. Staff only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GradebookTotals {
    pub published: GradebookOutcome,
    pub draft: GradebookOutcome,
}

/// A group of one student with what it adds up to (ADR-0089).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SummaryGroup {
    #[serde(flatten)]
    pub group: GradebookGroup,
    pub total: String,
    pub possible: String,
    pub capped: bool,
    /// The assignments of the group the student may see, in course order.
    pub assignments: Vec<Uuid>,
    /// Left out by the "best K" rule.
    pub dropped: Vec<Uuid>,
}

/// What the caller's published marks add up to, from published slots only (ADR-0089).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct MyGradeSummary {
    pub settings: GradebookSettings,
    /// The final total, never more than the target.
    pub total: String,
    /// Every point earned, before the target cuts it: a student with 116 of a target of 100 has a
    /// `total` of 100 and `earned` of 116.
    pub earned: String,
    pub possible: String,
    /// `null` when the course has no pass mark.
    pub passed: Option<bool>,
    /// The groups with something to show, in order. Assignments in no group are in `loose`.
    pub groups: Vec<SummaryGroup>,
    /// Assignments the student may see that are in no group; each counts as a plain one.
    pub loose: Vec<Uuid>,
    /// What those add up to.
    pub loose_total: String,
}

/// A proposed group: assignments whose titles share a stem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GradebookSuggestion {
    pub title: String,
    pub assignments: Vec<Uuid>,
}

/// Everything the Structure mode shows (ADR-0089). Staff and admins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GradebookStructure {
    pub settings: GradebookSettings,
    /// In the `custom` layout every group, empty ones included; in `course` the modules that have
    /// an assignment.
    pub groups: Vec<GradebookGroup>,
    /// Every assignment of the course in tree order, each with its group.
    pub columns: Vec<GradebookColumn>,
    pub possible: String,
    pub setup: GradebookSetup,
    /// Only in the `custom` layout, only for assignments in no group.
    pub suggestions: Vec<GradebookSuggestion>,
}

/// The rules of a group, and its title when it is one of the teacher's own. A `PUT` replaces the
/// rules, so every key has its default when absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Validate, ToSchema)]
pub struct GradebookGroupInput {
    /// 1 to 200 characters; required to create a group, refused for a module's group.
    #[serde(default)]
    #[garde(skip)]
    pub title: Option<String>,
    /// The most the group adds to the total, a decimal string above 0; `null` = no cap.
    #[serde(default, deserialize_with = "optional_points_text")]
    #[garde(skip)]
    pub cap: Option<String>,
    /// From 1 to 1000; `null` = all marks count.
    #[serde(default)]
    #[garde(skip)]
    pub best_k: Option<i32>,
    /// False = practice. Default true.
    #[serde(default = "yes")]
    #[garde(skip)]
    pub counts: bool,
    /// True = extra. Default false; a practice group cannot be bonus.
    #[serde(default)]
    #[garde(skip)]
    pub bonus: bool,
    /// Only when creating: assignments to put in the new group, at most 500.
    #[serde(default)]
    #[garde(skip)]
    pub assignments: Vec<Uuid>,
}

/// The key of a new group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CreatedGradebookGroup {
    pub key: Uuid,
}

/// Puts assignments in a group of the teacher's own, or in none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Validate, ToSchema)]
pub struct GradebookPlacement {
    /// At most 500, each once, all assignments of the course.
    #[garde(skip)]
    pub assignments: Vec<Uuid>,
    /// A group's key; `null` puts them back in "Not sorted".
    #[serde(default)]
    #[garde(skip)]
    pub group: Option<Uuid>,
}

#[cfg(test)]
mod tests;
