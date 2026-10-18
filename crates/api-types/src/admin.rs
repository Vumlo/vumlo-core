//! The administrator's read models and the audit log (ADR-0053). They carry ids, labels and
//! metadata, never sentences: the client words them.

use chrono::{DateTime, Utc};
use garde::Validate;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::{Role, UserStatus};

/// People by institution role, and how many are disabled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UserSummary {
    pub all: i64,
    pub student: i64,
    pub teacher: i64,
    pub admin: i64,
    pub disabled: i64,
}

/// A course as a line of a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CourseLine {
    pub course_id: Uuid,
    pub code: String,
    pub title: String,
}

/// A group, by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GroupName {
    pub id: Uuid,
    pub name: String,
}

/// What one person does in the institution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UserFacts {
    pub role: Role,
    pub status: UserStatus,
    pub created_at: DateTime<Utc>,
    /// `null` until the person signs in after the server learned to record it.
    pub last_login_at: Option<DateTime<Utc>>,
    pub groups: Vec<GroupName>,
    /// Regular courses the person owns.
    pub owns: Vec<CourseLine>,
    /// Regular courses where they are a teacher or an assistant but not the owner.
    pub teaches: Vec<CourseLine>,
    /// Published courses where they are a current student.
    pub studies: i64,
}

/// A person in a group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GroupMemberLine {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
    pub email: String,
    /// `null` for those who were in the group before the time was recorded.
    pub added_at: Option<DateTime<Utc>>,
}

/// A course that uses a group, or one of its subgroups.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct LinkedCourse {
    #[serde(flatten)]
    pub course: CourseLine,
    pub owner_name: String,
    pub owner_surname: String,
    pub linked_on: DateTime<Utc>,
}

/// A group, its people and the courses that use it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GroupOverview {
    pub group_id: Uuid,
    pub student_count: i64,
    pub course_count: i64,
    pub members: Vec<GroupMemberLine>,
    pub linked_courses: Vec<LinkedCourse>,
}

/// What one term holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TermStat {
    pub term_id: Uuid,
    /// Regular courses that use the term.
    pub courses: i64,
    /// Today, in the institution's time zone, falls in the term.
    pub current: bool,
    /// The term has ended.
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TermStats {
    pub items: Vec<TermStat>,
}

/// A course of the institution as the administrator lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AdminCourse {
    #[serde(flatten)]
    pub course: CourseLine,
    pub status: super::CourseStatus,
    pub owner_id: Uuid,
    pub owner_name: String,
    pub owner_surname: String,
    pub term_id: Option<Uuid>,
    pub term_name: Option<String>,
    pub student_count: i64,
}

/// What an audit entry is about, derived from its action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuditKind {
    Accounts,
    Owners,
    Teachers,
    Groups,
    Courses,
    Content,
    Grades,
    Terms,
    Other,
}

/// Where an audit row was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuditSource {
    Api,
    Cli,
}

/// The person who did it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AuditActor {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
}

/// One row of the audit log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AuditRecord {
    pub id: Uuid,
    pub at: DateTime<Utc>,
    pub source: AuditSource,
    pub kind: AuditKind,
    /// A dotted verb such as `course.owner.transfer`.
    pub action: String,
    /// `null` for a row the server CLI wrote.
    pub actor: Option<AuditActor>,
    pub resource_type: String,
    pub resource_id: Uuid,
    /// What the target is called now; `null` when it is gone.
    pub resource_label: Option<String>,
    /// Ids, roles and field names only.
    #[schema(value_type = Object)]
    pub meta: Map<String, Value>,
}

/// Filters of the audit log.
#[derive(Debug, Clone, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AuditQuery {
    /// Matches the actor's name, the action or the label of the target; at most 100 characters.
    #[garde(inner(length(chars, max = 100)))]
    pub q: Option<String>,
    #[garde(skip)]
    pub kind: Option<AuditKind>,
    /// How many days back, 1 to 365 (default 7).
    #[garde(inner(range(min = 1, max = 365)))]
    pub days: Option<u32>,
    #[garde(inner(length(min = 1, max = 100)))]
    pub cursor: Option<String>,
    /// Page size, 1 to 100 (default 50).
    #[garde(inner(range(min = 1, max = 100)))]
    pub limit: Option<u32>,
}
