//! Request and response types of the plugin API and of the reports it stores (ADR-0066).

use chrono::{DateTime, Utc};
use garde::Validate;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// The most a report's `data` may take when written as JSON.
pub const MAX_REPORT_DATA_BYTES: usize = 16 * 1024;

/// A submitted attempt as a plugin with `submissions:read` sees it: which work, whose, and what it
/// says. No draft, no score and no grade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PluginSubmission {
    /// The id the `submission.submitted` event carries as `submission_id`.
    pub id: Uuid,
    pub course_id: Uuid,
    pub assignment_id: Uuid,
    pub student_id: Uuid,
    pub attempt: i32,
    /// A block document for a `text` assignment, `{"answers": {...}}` for a quiz or a form,
    /// `{"files": [...], "comment"?}` for a file.
    #[schema(value_type = Object)]
    pub content: Map<String, Value>,
    pub submitted_at: DateTime<Utc>,
}

/// What a plugin says about a submitted attempt, an assignment or a course. It replaces the plugin's
/// earlier report on the subject under the same key.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct PutReportRequest {
    /// One line for people to read, 1 to 500 characters.
    #[garde(length(min = 1, max = 500))]
    pub summary: String,
    /// What a program may want: a JSON object of at most 16 KiB.
    #[schema(value_type = Object)]
    #[garde(custom(a_small_object))]
    pub data: Value,
}

fn a_small_object(data: &Value, _: &()) -> garde::Result {
    if !data.is_object() {
        return Err(garde::Error::new("must be a JSON object"));
    }
    if data.to_string().len() > MAX_REPORT_DATA_BYTES {
        return Err(garde::Error::new(format!(
            "must take at most {MAX_REPORT_DATA_BYTES} bytes as JSON"
        )));
    }
    Ok(())
}

/// A report of one plugin on a submitted attempt, an assignment or a course, for the staff who may see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PluginReport {
    /// The id the plugin gave in its manifest.
    pub plugin_id: String,
    /// The name the plugin gave this report: it keeps one report per key on a subject.
    pub key: String,
    pub summary: String,
    #[schema(value_type = Object)]
    pub data: Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PluginReports {
    pub items: Vec<PluginReport>,
}

/// Which place of the web app the pages are wanted for.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SlotQuery {
    /// A slot a plugin may name, e.g. `assignment.tab`. Another value is a `422`.
    #[garde(length(min = 1, max = 40))]
    pub slot: String,
}

/// A page that an enabled plugin offers in a slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PluginSlot {
    /// The id the plugin gave in its manifest.
    pub plugin_id: String,
    /// The plugin's own words, shown as the name of the tab. Plain text.
    pub title: String,
    /// Where the page is. The web app puts it in a sandboxed frame and nowhere else.
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PluginSlots {
    pub items: Vec<PluginSlot>,
}

/// Asks for the token a plugin page uses to learn who is looking and at what.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct LaunchRequest {
    #[garde(length(min = 1, max = 32))]
    pub plugin_id: String,
}

/// A token for the plugin's own server, valid for a minute. It says who looks at which assignment, in
/// the course's terms (`student` or `staff`), and is signed with the plugin's secret (ADR-0066).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Launch {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// Which scope of settings a plugin asks for.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SettingsQuery {
    /// Also the settings that are per course, for this course. Without it only the settings of the
    /// whole institution are returned.
    #[garde(skip)]
    pub course_id: Option<Uuid>,
}

/// The settings of a plugin as an administrator configured them, with the defaults of its manifest
/// where nothing was set. A setting with neither is not in `values`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PluginSettings {
    /// Setting key to value: a string, a number, a boolean or a list. Secrets are in clear: the
    /// plugin is who they were entered for.
    #[schema(value_type = Object)]
    pub values: Map<String, Value>,
}

// ---------------------------------------------------------------------------------------------
// The read surface of the plugin API (ADR-0074)
// ---------------------------------------------------------------------------------------------

/// How a plugin pages through a list. The cursor is opaque: pass back `next_cursor` unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PluginListQuery {
    #[garde(inner(length(min = 1, max = 100)))]
    pub cursor: Option<String>,
    /// Page size, 1 to 100 (default 50).
    #[garde(inner(range(min = 1, max = 100)))]
    pub limit: Option<u32>,
    /// Only what changed at or after this time (RFC 3339), so a plugin can catch up after events it
    /// missed. What counts as a change is said on each list.
    #[garde(skip)]
    pub updated_since: Option<DateTime<Utc>>,
}

/// The assignments of a course, or of every course the plugin reaches.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PluginAssignmentsQuery {
    /// One course. Without it, every course the plugin was granted.
    #[garde(skip)]
    pub course_id: Option<Uuid>,
    #[garde(inner(length(min = 1, max = 100)))]
    pub cursor: Option<String>,
    /// Page size, 1 to 100 (default 50).
    #[garde(inner(range(min = 1, max = 100)))]
    pub limit: Option<u32>,
    /// Only assignments, titles or settings changed at or after this time (RFC 3339).
    #[garde(skip)]
    pub updated_since: Option<DateTime<Utc>>,
}

/// The published grades of a course, of every course the plugin reaches, or of one assignment.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PluginGradesQuery {
    #[garde(skip)]
    pub course_id: Option<Uuid>,
    #[garde(skip)]
    pub assignment_id: Option<Uuid>,
    #[garde(inner(length(min = 1, max = 100)))]
    pub cursor: Option<String>,
    /// Page size, 1 to 100 (default 50).
    #[garde(inner(range(min = 1, max = 100)))]
    pub limit: Option<u32>,
    /// Only grades published at or after this time (RFC 3339).
    #[garde(skip)]
    pub updated_since: Option<DateTime<Utc>>,
}

/// The people of a course.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PluginMembersQuery {
    /// Only people with this role in the course.
    #[garde(skip)]
    pub role: Option<crate::CourseRole>,
    #[garde(inner(length(min = 1, max = 100)))]
    pub cursor: Option<String>,
    /// Page size, 1 to 100 (default 50).
    #[garde(inner(range(min = 1, max = 100)))]
    pub limit: Option<u32>,
}

/// The most user ids one call to `users/contact` takes.
pub const MAX_CONTACT_IDS: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PluginContactQuery {
    /// User ids, separated by commas: at most 100.
    #[garde(length(min = 36, max = 4000))]
    pub ids: String,
}

/// An assignment as a student can see it: open, with its published title.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PluginAssignment {
    pub id: Uuid,
    pub course_id: Uuid,
    /// `text`, `no_submission`, `quiz`, `form` or `file`.
    pub kind: String,
    pub title: String,
    pub due_at: Option<DateTime<Utc>>,
    /// A decimal number as text.
    pub max_points: String,
    /// The fields of a `form` (`id`, `type`, `label`, `help`, `required`, `options`, `rules`); absent
    /// for the other types.
    #[schema(value_type = Option<Vec<Object>>)]
    pub fields: Option<Vec<Value>>,
    /// When the assignment, its title or its settings last changed.
    pub updated_at: DateTime<Utc>,
}

/// The values a student gave in a `form`, by field id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PluginFormAnswers {
    pub submission_id: Uuid,
    pub course_id: Uuid,
    pub assignment_id: Uuid,
    pub student_id: Uuid,
    #[schema(value_type = Object)]
    pub answers: Map<String, Value>,
}

/// A submitted attempt in the list of an assignment, without its content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PluginSubmissionItem {
    pub id: Uuid,
    pub student_id: Uuid,
    pub attempt: i32,
    pub submitted_at: DateTime<Utc>,
    pub is_late: bool,
    /// When the grade of this student for the assignment was published; `null` while it is not.
    pub graded_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PluginMember {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
    pub role: crate::CourseRole,
    /// `direct`, or `group` when the student is on the roster through a linked group.
    pub source: String,
}

/// A published grade. A draft is never here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PluginGrade {
    pub course_id: Uuid,
    pub assignment_id: Uuid,
    pub student_id: Uuid,
    /// A decimal number as text; `null` when only feedback was published.
    pub score: Option<String>,
    pub max_points: String,
    /// Who published the grade; `null` for a grade the system published.
    pub published_by: Option<Uuid>,
    pub published_at: DateTime<Utc>,
}

/// The questions of a quiz as a student sees them: no `correct`, `answer`, `tolerance`,
/// `accepted`, `explanation`, `partial_credit` or `grading_notes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PluginQuiz {
    pub assignment_id: Uuid,
    pub course_id: Uuid,
    pub max_points: String,
    #[schema(value_type = Object)]
    pub config: Map<String, Value>,
}

/// A submitted attempt at a quiz.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PluginAttempt {
    pub id: Uuid,
    pub student_id: Uuid,
    pub attempt: i32,
    /// The score of each question by question id, as decimal text.
    #[schema(value_type = Object)]
    pub scores: Map<String, Value>,
    /// When the attempt was first saved as a draft.
    pub started_at: DateTime<Utc>,
    pub submitted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PluginContact {
    pub user_id: Uuid,
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PluginContacts {
    /// Those of the ids who are in a course the plugin reaches. The others are left out.
    pub items: Vec<PluginContact>,
}

/// The announcements of a course or of every course the plugin reaches.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PluginAnnouncementsQuery {
    /// One course. Without it, every course the plugin was granted.
    #[garde(skip)]
    pub course_id: Option<Uuid>,
    #[garde(inner(length(min = 1, max = 100)))]
    pub cursor: Option<String>,
    /// Page size, 1 to 100 (default 50).
    #[garde(inner(range(min = 1, max = 100)))]
    pub limit: Option<u32>,
    /// Only announcements published or changed at or after this time (RFC 3339).
    #[garde(skip)]
    pub updated_since: Option<DateTime<Utc>>,
}

/// What was deleted in a course or in every course the plugin reaches (ADR-0081).
#[derive(Debug, Clone, Serialize, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PluginDeletedQuery {
    /// Only what was deleted at or after this time (RFC 3339). The time of the last item of the
    /// previous read is a good one: the list is inclusive, and reading an id twice does no harm.
    /// Not older than `plugins.tombstone_days`: a plugin that was away longer is told `410`.
    #[garde(skip)]
    pub deleted_since: DateTime<Utc>,
    /// One course. Without it, every course the plugin was granted. A course that is itself gone
    /// is not a `404`: what was deleted with it is listed.
    #[garde(skip)]
    pub course_id: Option<Uuid>,
    #[garde(inner(length(min = 1, max = 100)))]
    pub cursor: Option<String>,
    /// Page size, 1 to 100 (default 50).
    #[garde(inner(range(min = 1, max = 100)))]
    pub limit: Option<u32>,
}

/// Something that was shown to a plugin and has been deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PluginDeletion {
    pub id: Uuid,
    pub course_id: Uuid,
    pub deleted_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests;
