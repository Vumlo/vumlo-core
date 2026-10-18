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
