use serde_json::json;

use super::*;

fn save(value: Value) -> Result<SaveGradeRequest, serde_json::Error> {
    serde_json::from_value(value)
}

#[test]
fn a_save_needs_both_keys_and_null_clears() {
    assert!(save(json!({})).is_err());
    assert!(save(json!({"score": "5"})).is_err());
    assert!(save(json!({"feedback": "ok"})).is_err());
    let cleared = save(json!({"score": null, "feedback": null})).unwrap();
    assert_eq!((cleared.score, cleared.feedback), (None, None));
    let set = save(json!({"score": "7.5", "feedback": "good"})).unwrap();
    assert_eq!(set.score.as_deref(), Some("7.5"));
    assert_eq!(set.feedback, Some(json!("good")));
}

#[test]
fn a_score_that_is_not_text_becomes_the_rejected_empty_string() {
    for sent in [
        json!(5),
        json!(7.5),
        json!(true),
        json!([1]),
        json!({"a": 1}),
    ] {
        let request = save(json!({"score": sent, "feedback": null})).unwrap();
        assert_eq!(request.score.as_deref(), Some(""), "{sent}");
    }
    let kept = save(json!({"score": "1.005", "feedback": null})).unwrap();
    assert_eq!(
        kept.score.as_deref(),
        Some("1.005"),
        "text is judged by the server"
    );
}

#[test]
fn a_feedback_that_is_not_text_is_kept_for_the_server_to_reject() {
    for sent in [json!(5), json!(true), json!([]), json!({})] {
        let request = save(json!({"score": null, "feedback": sent.clone()})).unwrap();
        assert_eq!(request.feedback, Some(sent));
    }
}

fn publish(value: Value) -> Result<PublishGradesRequest, serde_json::Error> {
    serde_json::from_value(value)
}

#[test]
fn publish_takes_all_or_a_list_and_the_body_is_required() {
    assert!(publish(json!({})).is_err());
    assert!(publish(json!({"students": null})).is_err());
    for bad in [
        json!("everyone"),
        json!("ALL"),
        json!(1),
        json!(["not-a-uuid"]),
        json!({}),
    ] {
        assert!(publish(json!({"students": bad.clone()})).is_err(), "{bad}");
    }
    let all = publish(json!({"students": "all"})).unwrap();
    assert_eq!(all.students, PublishStudents::All(AllStudents::All));
    let id = Uuid::now_v7();
    let some = publish(json!({"students": [id]})).unwrap();
    assert_eq!(some.students, PublishStudents::Ids(vec![id]));
    assert_eq!(
        serde_json::to_value(&all).unwrap(),
        json!({"students": "all"})
    );
}

#[test]
fn a_publish_list_is_capped() {
    let ids = |n: usize| PublishStudents::Ids((0..n).map(|_| Uuid::now_v7()).collect());
    let request = |students| PublishGradesRequest { students };
    assert!(request(ids(MAX_PUBLISH_STUDENTS)).validate().is_ok());
    assert!(request(ids(MAX_PUBLISH_STUDENTS + 1)).validate().is_err());
    assert!(
        request(PublishStudents::All(AllStudents::All))
            .validate()
            .is_ok()
    );
}

#[test]
fn a_student_view_has_exactly_the_published_fields() {
    let grade = MyGrade {
        assignment_id: Uuid::nil(),
        title: "t".into(),
        max_points: "10.00".into(),
        is_bonus: false,
        score: Some("7.50".into()),
        feedback: None,
        published_at: DateTime::UNIX_EPOCH,
    };
    let value = serde_json::to_value(grade).unwrap();
    let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "assignment_id",
            "feedback",
            "is_bonus",
            "max_points",
            "published_at",
            "score",
            "title"
        ]
    );
    assert!(!value.to_string().contains("draft"));
}

#[test]
fn a_gradebook_cell_flattens_the_grade_fields() {
    let cell = GradebookCell {
        assignment_id: Uuid::nil(),
        cell: GradeCell {
            draft_score: Some("8.00".into()),
            draft_feedback: None,
            published_score: None,
            published_feedback: None,
            has_unpublished_changes: true,
        },
    };
    let value = serde_json::to_value(cell).unwrap();
    assert_eq!(value["draft_score"], json!("8.00"));
    assert_eq!(value["has_unpublished_changes"], json!(true));
    assert_eq!(value["published_score"], Value::Null);
    assert!(value.get("cell").is_none());
}
