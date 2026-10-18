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
