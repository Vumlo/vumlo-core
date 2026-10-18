//! Request and response types of terms, groups and subgroups (ADR-0032) and of the institution
//! profile (ADR-0044). Dates are `YYYY-MM-DD`.

use chrono::{DateTime, NaiveDate, Utc};
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// The interface language an institution starts in (ADR-0043).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    En,
    Uk,
}

/// How points relate to the grade (ADR-0022).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum GradingScale {
    /// At most 12 points per assignment.
    Twelve,
    /// The regular assignments of a course add up to at most 100 points.
    Hundred,
}

/// What kind of institution the installation serves (ADR-0031).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum InstitutionMode {
    School,
    University,
}

/// The public profile of the institution: what a page needs before anyone signs in (ADR-0044).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Institution {
    pub name: String,
    /// For narrow places such as the navbar.
    pub short_name: String,
    pub language: Language,
    /// An IANA zone name, e.g. `Europe/Kyiv`.
    pub time_zone: String,
    pub grading_scale: GradingScale,
}

/// What an admin sees: the public profile and the settings that are not public. Read-only; they
/// change in the config file (ADR-0044).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AdminInstitution {
    #[serde(flatten)]
    pub institution: Institution,
    pub mode: InstitutionMode,
    /// The email domains self-registration accepts (ADR-0017).
    pub allowed_domains: Vec<String>,
}

/// The most users one request may add to a group or a subgroup.
pub const MAX_MEMBERS_PER_REQUEST: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Term {
    pub id: Uuid,
    pub name: String,
    pub starts_on: NaiveDate,
    pub ends_on: NaiveDate,
}

/// `ends_on` must not be before `starts_on`.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateTermRequest {
    /// 1 to 100 characters after trimming; unique.
    #[garde(length(chars, min = 1, max = 100))]
    pub name: String,
    #[garde(skip)]
    pub starts_on: NaiveDate,
    #[garde(skip)]
    pub ends_on: NaiveDate,
}

/// Only the fields present change; the dates are checked together after the merge.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateTermRequest {
    #[garde(inner(length(chars, min = 1, max = 100)))]
    pub name: Option<String>,
    #[garde(skip)]
    pub starts_on: Option<NaiveDate>,
    #[garde(skip)]
    pub ends_on: Option<NaiveDate>,
}

/// A cohort: school class 7-A, university group PZ-31.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Group {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

/// A part of one group. One level only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Subgroup {
    pub id: Uuid,
    pub group_id: Uuid,
    pub name: String,
}

/// The name of a group or a subgroup, to create or to rename.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct NameRequest {
    /// 1 to 100 characters after trimming. Unique among groups, or inside the group for a
    /// subgroup.
    #[garde(length(chars, min = 1, max = 100))]
    pub name: String,
}

/// A member of a group or a subgroup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GroupMember {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct AddMembersRequest {
    /// 1 to 1000 user ids. All or nothing: one bad entry rejects the request, and the `422`
    /// names its position as `user_ids[i]`.
    #[garde(length(min = 1, max = MAX_MEMBERS_PER_REQUEST))]
    pub user_ids: Vec<Uuid>,
}

/// The members a group must have after a rollover (ADR-0037).
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct ReplaceMembersRequest {
    /// 0 to 1000 user ids: exactly the students the group has afterwards. All or nothing: an entry
    /// that is no user or not a student rejects the request, and the `422` names its position as
    /// `user_ids[i]`. An empty list empties the group.
    #[garde(length(max = MAX_MEMBERS_PER_REQUEST))]
    pub user_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ReplaceMembersResponse {
    /// The users who were not members before.
    pub added: Vec<Uuid>,
    /// The members who are not in the new list.
    pub removed: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct RemoveMembersRequest {
    /// 1 to 1000 user ids. Ids that are not members are skipped.
    #[garde(length(min = 1, max = MAX_MEMBERS_PER_REQUEST))]
    pub user_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RemoveMembersResponse {
    /// The users who were members and are not any more.
    pub removed: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AddMembersResponse {
    /// The users who were not members before. Empty when all of them already were.
    pub added: Vec<Uuid>,
}

#[cfg(test)]
mod tests;
