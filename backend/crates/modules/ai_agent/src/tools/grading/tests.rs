use serde_json::json;

use super::*;

fn parse(name: &str, args: Value) -> Result<Write, String> {
    Write::parse(name, &args.to_string())
}

#[test]
fn a_grade_needs_both_fields_stated_because_saving_replaces_the_whole_draft() {
    let (a, s) = (Uuid::now_v7(), Uuid::now_v7());
    let missing = parse(
        "set_grade",
        json!({"assignment_id": a, "student_id": s, "score": "8"}),
    );
    assert!(missing.unwrap_err().contains("feedback is required"));
    let missing = parse(
        "set_grade",
        json!({"assignment_id": a, "student_id": s, "feedback": "x"}),
    );
    assert!(missing.unwrap_err().contains("score is required"));

    // null is a choice: it leaves the field empty. A number is as good as text.
    let ok = parse(
        "set_grade",
        json!({"assignment_id": a, "student_id": s, "score": 8.5, "feedback": null}),
    )
    .unwrap();
    let Write::SetGrade {
        score, feedback, ..
    } = ok
    else {
        panic!("{ok:?}")
    };
    assert_eq!((score.as_deref(), feedback), (Some("8.5"), None));
}

#[test]
fn a_deadline_must_say_null_to_remove_and_a_wrong_shape_is_refused() {
    let a = Uuid::now_v7();
    assert!(
        parse("set_deadline", json!({"assignment_id": a}))
            .unwrap_err()
            .contains("due_at is required")
    );
    let cleared = parse("set_deadline", json!({"assignment_id": a, "due_at": null})).unwrap();
    assert!(matches!(cleared, Write::SetDeadline { due_at: None, .. }));
    let set = parse(
        "set_deadline",
        json!({"assignment_id": a, "due_at": "2026-12-01T23:59:00+02:00"}),
    )
    .unwrap();
    assert!(
        matches!(set, Write::SetDeadline { due_at: Some(d), .. } if d.to_rfc3339() == "2026-12-01T21:59:00+00:00")
    );

    for bad in [
        json!({"assignment_id": "not-a-uuid", "due_at": null}),
        json!({"assignment_id": a, "due_at": "tomorrow"}),
        json!({"assignment_id": a, "due_at": null, "extra": 1}),
        json!([1, 2]),
    ] {
        assert!(parse("set_deadline", bad.clone()).is_err(), "{bad}");
    }
    assert!(Write::parse("set_deadline", "not json").is_err());
}

#[test]
fn what_is_stored_is_what_runs_and_the_tool_name_survives_the_round_trip() {
    let a = Uuid::now_v7();
    for (name, args) in [
        ("publish_grades", json!({"assignment_id": a})),
        ("set_deadline", json!({"assignment_id": a, "due_at": null})),
    ] {
        let write = parse(name, args).unwrap();
        assert_eq!(write.name(), name);
        let stored = serde_json::to_string(&write).unwrap();
        let again: Write = serde_json::from_str(&stored).unwrap();
        assert_eq!(again.name(), name);
    }
    assert_eq!(NAMES, ["set_grade", "publish_grades", "set_deadline"]);
}

#[test]
fn refusals_are_explained_in_the_services_words() {
    assert_eq!(
        explain(&ApiError::Conflict("the course is archived")),
        "the course is archived"
    );
    let e = ApiError::invalid_field("score", "must be at most 20");
    assert_eq!(explain(&e), "score: must be at most 20");
    assert!(explain(&ApiError::Internal).contains("could not be made"));
}
