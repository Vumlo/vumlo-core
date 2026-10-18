//! Group links, exclusions and course editing (ADR-0033).

use chrono::{DateTime, Utc};
use garde::Validate;
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Where a roster entry comes from: a membership written on the course, or a linked group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum MemberSource {
    /// A membership written on the course. It wins over a group.
    Direct,
    /// A student through a linked group or subgroup. Always the `student` role.
    Group,
}

/// A live link of a course to a whole group, or to one subgroup of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CourseGroupLink {
    pub id: Uuid,
    pub course_id: Uuid,
    pub group_id: Uuid,
    /// Set when only this subgroup is linked; `null` links the whole group.
    pub subgroup_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct LinkGroupRequest {
    #[garde(skip)]
    pub group_id: Uuid,
    /// A subgroup of `group_id`; absent or `null` links the whole group.
    #[serde(default)]
    #[garde(skip)]
    pub subgroup_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SubgroupOption {
    pub id: Uuid,
    pub name: String,
}

/// A group a course can be linked to, with its subgroups. Ids and names only: no members.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GroupOption {
    pub id: Uuid,
    pub name: String,
    pub subgroups: Vec<SubgroupOption>,
}

/// A person taken off the roster of one course although a linked group would put them on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Exclusion {
    pub user_id: Uuid,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
}

/// A key that may be absent (change nothing), `null` (clear) or a value (set).
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Only the fields present change.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateCourseRequest {
    /// 1 to 200 characters after trimming.
    #[garde(inner(length(chars, min = 1, max = 200)))]
    pub title: Option<String>,
    /// An existing term; `null` clears it; absent changes nothing. A template has no term.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<Uuid>)]
    #[garde(skip)]
    pub term_id: Option<Option<Uuid>>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn update(value: serde_json::Value) -> UpdateCourseRequest {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_term_can_be_absent_null_or_a_value() {
        let id = Uuid::now_v7();
        assert_eq!(update(json!({})).term_id, None);
        assert_eq!(update(json!({"term_id": null})).term_id, Some(None));
        assert_eq!(update(json!({"term_id": id})).term_id, Some(Some(id)));
        assert!(serde_json::from_value::<UpdateCourseRequest>(json!({"term_id": 5})).is_err());
    }

    #[test]
    fn a_title_is_bounded_when_sent() {
        assert!(update(json!({})).validate().is_ok());
        assert!(update(json!({"title": ""})).validate().is_err());
        assert!(
            update(json!({"title": "x".repeat(201)}))
                .validate()
                .is_err()
        );
        assert!(update(json!({"title": "Algebra"})).validate().is_ok());
    }

    #[test]
    fn a_link_names_a_group_and_optionally_a_subgroup() {
        let group = Uuid::now_v7();
        let whole: LinkGroupRequest = serde_json::from_value(json!({"group_id": group})).unwrap();
        assert_eq!(whole.subgroup_id, None);
        assert!(serde_json::from_value::<LinkGroupRequest>(json!({})).is_err());
    }

    #[test]
    fn a_source_is_lowercase() {
        assert_eq!(
            serde_json::to_value(MemberSource::Direct).unwrap(),
            json!("direct")
        );
        assert_eq!(
            serde_json::to_value(MemberSource::Group).unwrap(),
            json!("group")
        );
    }
}
