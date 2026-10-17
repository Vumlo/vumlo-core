//! Request and response types of terms, groups and subgroups (ADR-0032). Dates are `YYYY-MM-DD`.

use chrono::{DateTime, NaiveDate, Utc};
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AddMembersResponse {
    /// The users who were not members before. Empty when all of them already were.
    pub added: Vec<Uuid>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn dates_are_plain_iso_dates() {
        let term: CreateTermRequest = serde_json::from_value(
            json!({"name": "Fall 2026", "starts_on": "2026-09-01", "ends_on": "2027-01-31"}),
        )
        .unwrap();
        assert_eq!(term.starts_on, NaiveDate::from_ymd_opt(2026, 9, 1).unwrap());
        assert!(
            serde_json::from_value::<CreateTermRequest>(
                json!({"name": "x", "starts_on": "2026-09-01T00:00:00Z", "ends_on": "2027-01-31"})
            )
            .is_err()
        );
    }

    #[test]
    fn a_member_list_has_one_to_a_thousand_ids() {
        let request = |n: usize| AddMembersRequest {
            user_ids: (0..n).map(|_| Uuid::now_v7()).collect(),
        };
        assert!(request(0).validate().is_err());
        assert!(request(1).validate().is_ok());
        assert!(request(MAX_MEMBERS_PER_REQUEST).validate().is_ok());
        assert!(request(MAX_MEMBERS_PER_REQUEST + 1).validate().is_err());
    }

    #[test]
    fn names_are_one_to_a_hundred_characters() {
        let name = |text: &str| NameRequest {
            name: text.to_owned(),
        };
        assert!(name("").validate().is_err());
        assert!(name(&"ł".repeat(100)).validate().is_ok());
        assert!(name(&"ł".repeat(101)).validate().is_err());
    }
}
