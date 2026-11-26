use serde_json::{Value, json};

use super::*;
use crate::EffectiveProfiles;

fn doc(words: usize) -> Map<String, Value> {
    let text = vec!["word"; words].join(" ");
    let value = json!({
        "v": 1,
        "blocks": [{"id": "p", "type": "paragraph", "content": [{"type": "text", "text": text}]}],
        "attachments": []
    });
    value.as_object().unwrap().clone()
}

#[test]
fn reading_time_is_the_words_over_200_rounded_up_and_at_least_one() {
    for (words, minutes) in [(0, 1), (1, 1), (200, 1), (201, 2), (400, 2), (401, 3)] {
        assert_eq!(read_minutes(&doc(words)), Some(minutes), "{words} words");
        assert_eq!(Lecture.reading_minutes(&doc(words)), Some(minutes));
    }
}

#[test]
fn a_body_that_is_not_a_document_has_no_reading_time() {
    let body = json!({"nonsense": true});
    assert_eq!(read_minutes(body.as_object().unwrap()), None);
}

#[test]
fn a_lecture_is_a_full_document_with_no_points_and_is_marked_by_hand() {
    assert_eq!(Lecture.id(), "lecture");
    assert!(Lecture.graded().is_none());
    assert_eq!(Lecture.completion(), CompletionPolicy::Marked);
    let capabilities = Lecture.capabilities();
    assert!(!capabilities.graded && !capabilities.submissions && !capabilities.auto_graded);
    // The body it starts with is a document it accepts.
    let start = Lecture.default_body();
    let profiles = EffectiveProfiles::unrestricted();
    assert_eq!(
        Lecture
            .validate_body(&Value::Object(start.clone()), &profiles)
            .unwrap(),
        start
    );
    assert!(Lecture.validate_body(&json!({"v": 2}), &profiles).is_err());
    assert_eq!(Lecture.body_file_refs(&start), Ok(Vec::new()));
    assert_eq!(Lecture.check_publishable(&start), Ok(()));
}
