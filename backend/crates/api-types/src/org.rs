//! Request and response types of terms, groups and subgroups (ADR-0032) and of the institution
//! profile (ADR-0044). Dates are `YYYY-MM-DD`.

use chrono::{DateTime, NaiveDate, Utc};
use garde::Validate;
use serde::{Deserialize, Deserializer, Serialize};
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

/// A kind of installer of the desktop app (ADR-0064): the systems of ADR-0063's installers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AppPlatform {
    Windows,
    MacosArm,
    MacosIntel,
    LinuxDeb,
    LinuxAppimage,
}

/// Where one installer of the institution's desktop app can be downloaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AppDownload {
    pub platform: AppPlatform,
    /// An `https` address.
    pub url: String,
}

/// The whole set of download links: what is not in the list is removed. An empty list switches the
/// download hint and page off.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct SetAppDownloadsRequest {
    /// At most one link per platform, each at most 2048 characters of an `https` address.
    #[garde(length(max = 5))]
    pub downloads: Vec<AppDownload>,
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
    /// The links to the desktop app's installers, in the order platforms are listed above. Empty: the
    /// institution has no app to offer (ADR-0064).
    pub app_downloads: Vec<AppDownload>,
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

/// A unit of the institution: a node of a tree (ADR-0094).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Unit {
    pub id: Uuid,
    /// `null` for a top-level unit.
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

/// What a unit role can be switched to allow. Viewing is implicit to any role and has no switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnitAbility {
    ViewGrades,
    EditContent,
    Grade,
    ManageRoster,
    ManageCourses,
    ManageUnits,
}

/// A unit the caller sees, with what their roles let them do on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UnitSummary {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub name: String,
    /// The switches the caller holds on this unit, the union over every role that covers it. An
    /// administrator holds all of them. Viewing is implicit to being listed.
    pub abilities: Vec<UnitAbility>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateUnitRequest {
    /// 1 to 100 characters after trimming. Unique among the units with the same parent.
    #[garde(length(chars, min = 1, max = 100))]
    pub name: String,
    /// The unit to create this one under. Omit for a top-level unit (administrators only).
    #[serde(default)]
    #[garde(skip)]
    pub parent_id: Option<Uuid>,
}

/// Only the fields present change.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateUnitRequest {
    #[garde(inner(length(chars, min = 1, max = 100)))]
    pub name: Option<String>,
    /// The new parent; `null` moves the unit to the top (administrators only); absent changes nothing.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<Uuid>)]
    #[garde(skip)]
    pub parent_id: Option<Option<Uuid>>,
}

/// A key that may be absent (change nothing), `null` (clear) or a value (set).
pub(crate) fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(deserializer).map(Some)
}

/// A person's role on a unit. It covers the unit and every unit below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnitRole {
    Manager,
    Viewer,
}

/// Someone who holds a role directly on a unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UnitRoleEntry {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
    pub email: String,
    pub role: UnitRole,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct SetUnitRoleRequest {
    #[garde(skip)]
    pub role: UnitRole,
}

/// The switches of each unit role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UnitRoleAbilities {
    pub manager: Vec<UnitAbility>,
    pub viewer: Vec<UnitAbility>,
}

/// The switches a unit role has afterwards: exactly these. An empty list leaves the role able to
/// view only. A repeated ability counts once.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct SetUnitRoleAbilitiesRequest {
    #[garde(skip)]
    pub abilities: Vec<UnitAbility>,
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
