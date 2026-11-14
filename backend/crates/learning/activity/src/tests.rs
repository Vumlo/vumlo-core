//! The contract data of an activity type. The document model has its own tests in `document/`.

use serde_json::Map;

use super::*;

#[test]
fn a_blank_content_says_so() {
    let content = ValidContent::new(Map::new(), true);
    assert!(content.is_blank());
    assert!(content.as_body().is_empty());
}

#[test]
fn claims_are_empty_by_default() {
    let mut claims = Claims::default();
    assert!(claims.is_empty());
    claims.files.push(uuid::Uuid::nil());
    assert!(!claims.is_empty());
}

#[test]
fn scores_are_stored_with_two_decimals() {
    let grade = AutoGrade {
        scores: [("q1".to_owned(), "2.5".parse().unwrap())].into(),
        total: "2.5".parse().unwrap(),
        complete: true,
    };
    assert_eq!(grade.scores_json()["q1"], "2.50");
}
