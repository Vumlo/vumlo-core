use serde_json::json;

use super::*;

#[test]
fn publish_at_distinguishes_absent_null_and_a_value() {
    let absent: UpdateModuleRequest = serde_json::from_value(json!({})).unwrap();
    assert_eq!(absent.publish_at, None);
    let cleared: UpdateModuleRequest = serde_json::from_value(json!({"publish_at": null})).unwrap();
    assert_eq!(cleared.publish_at, Some(None));
    let set: UpdateItemRequest =
        serde_json::from_value(json!({"publish_at": "2030-01-02T03:04:05Z"})).unwrap();
    assert!(matches!(set.publish_at, Some(Some(_))));
}

#[test]
fn cascade_is_required() {
    assert!(serde_json::from_value::<PublishModuleRequest>(json!({})).is_err());
    let request: PublishModuleRequest = serde_json::from_value(json!({"cascade": false})).unwrap();
    assert!(!request.cascade);
}

#[test]
fn a_body_is_read_as_a_json_value_and_null_is_absent() {
    // What a body may contain is decided by the document validator, which reports a path.
    for body in [json!([1]), json!("text"), json!({"a": 1})] {
        let create = json!({"kind": "text", "title": "t", "body": body});
        let create: CreateItemRequest = serde_json::from_value(create).unwrap();
        assert_eq!(create.body, Some(body.clone()));
        let update: UpdateItemRequest = serde_json::from_value(json!({"body": body})).unwrap();
        assert_eq!(update.body, Some(body));
    }
    let absent: UpdateItemRequest = serde_json::from_value(json!({"body": null})).unwrap();
    assert_eq!(absent.body, None);
    let omitted: CreateItemRequest =
        serde_json::from_value(json!({"kind": "text", "title": "t"})).unwrap();
    assert_eq!(omitted.body, None);
}

#[test]
fn the_item_kinds_are_text_and_assignment() {
    assert_eq!(serde_json::to_value(ItemKind::Text).unwrap(), json!("text"));
    let old = json!({"kind": "lesson", "title": "t"});
    assert!(serde_json::from_value::<CreateItemRequest>(old).is_err());
}

#[test]
fn a_tree_entry_flattens_the_module_fields() {
    let module = Module {
        id: Uuid::nil(),
        course_id: Uuid::nil(),
        parent_id: None,
        position: 1,
        title: "t".into(),
        status: ContentStatus::Draft,
        publish_at: None,
        created_at: DateTime::UNIX_EPOCH,
        updated_at: DateTime::UNIX_EPOCH,
    };
    let value = serde_json::to_value(ContentModule {
        module,
        submodules: vec![],
        items: vec![],
    })
    .unwrap();
    assert_eq!(value["title"], json!("t"));
    assert_eq!(value["submodules"], json!([]));
}

fn new_assignment(value: Value) -> NewAssignment {
    serde_json::from_value(value).unwrap()
}

#[test]
fn a_new_assignment_takes_defaults_for_what_is_missing() {
    let empty = new_assignment(json!({}));
    assert_eq!(empty.assignment_type, "");
    assert_eq!(empty.max_points, "");
    assert_eq!(empty.config, None);
    assert!(!empty.is_bonus);

    let only_type = new_assignment(json!({"type": "text"}));
    assert_eq!(only_type.assignment_type, "text");
    assert_eq!(only_type.max_points, "");

    let only_points = new_assignment(json!({"max_points": "20"}));
    assert_eq!(only_points.assignment_type, "");
    assert_eq!(only_points.max_points, "20");
}

#[test]
fn a_new_assignments_points_that_are_not_text_become_the_rejected_empty_string() {
    for sent in [
        json!(20),
        json!(7.5),
        json!(true),
        json!([1]),
        json!({"a": 1}),
        json!(null),
    ] {
        let assignment = new_assignment(json!({"type": "text", "max_points": sent}));
        assert_eq!(assignment.max_points, "", "{sent}");
    }
    let kept = new_assignment(json!({"type": "text", "max_points": "1.005"}));
    assert_eq!(
        kept.max_points, "1.005",
        "text is passed on for the server to judge"
    );
}

#[test]
fn an_update_reads_points_text_and_treats_null_as_absent() {
    let update = |value: Value| serde_json::from_value::<UpdateItemRequest>(value).unwrap();
    assert_eq!(update(json!({})).max_points, None);
    assert_eq!(update(json!({"max_points": null})).max_points, None);
    assert_eq!(
        update(json!({"max_points": "12.5"})).max_points.as_deref(),
        Some("12.5")
    );
    for sent in [json!(12), json!(false), json!([]), json!({})] {
        assert_eq!(
            update(json!({"max_points": sent})).max_points.as_deref(),
            Some(""),
            "{sent}"
        );
    }
    let nulls = update(json!({"config": null, "is_bonus": null}));
    assert_eq!(nulls.config, None);
    assert_eq!(nulls.is_bonus, None);
    let set = update(json!({"config": {}, "is_bonus": true}));
    assert_eq!(set.config, Some(Map::new()));
    assert_eq!(set.is_bonus, Some(true));
}

#[test]
fn a_text_item_serializes_without_an_assignment_key() {
    let item = |assignment| Item {
        id: Uuid::nil(),
        module_id: Uuid::nil(),
        course_id: Uuid::nil(),
        position: 1,
        kind: ItemKind::Text,
        title: "t".into(),
        body: Map::new(),
        status: ContentStatus::Draft,
        publish_at: None,
        assignment,
        created_at: DateTime::UNIX_EPOCH,
        updated_at: DateTime::UNIX_EPOCH,
    };
    let text = serde_json::to_value(item(None)).unwrap();
    assert!(text.get("assignment").is_none());

    let assignment = serde_json::to_value(item(Some(Assignment {
        assignment_type: "text".into(),
        config: Map::new(),
        max_points: "20.00".into(),
        is_bonus: false,
        due_at: None,
        on_deadline: None,
        accept_late: None,
        late_until: None,
        max_attempts: None,
        class_on: None,
    })))
    .unwrap();
    assert_eq!(
        assignment["assignment"],
        json!({
            "type": "text", "config": {}, "max_points": "20.00", "is_bonus": false,
            "due_at": null, "on_deadline": null, "accept_late": null, "late_until": null,
            "max_attempts": null, "class_on": null
        })
    );
}

#[test]
fn an_update_tells_absent_from_null_for_every_setting() {
    let update = |value: Value| serde_json::from_value::<UpdateItemRequest>(value).unwrap();
    let absent = update(json!({}));
    assert_eq!(absent.due_at, None);
    assert_eq!(absent.on_deadline, None);
    assert_eq!(absent.accept_late, None);
    assert_eq!(absent.late_until, None);
    assert_eq!(absent.max_attempts, None);

    let cleared = update(json!({
        "due_at": null, "on_deadline": null, "accept_late": null,
        "late_until": null, "max_attempts": null
    }));
    assert_eq!(cleared.due_at, Some(None));
    assert_eq!(cleared.on_deadline, Some(None));
    assert_eq!(cleared.accept_late, Some(None));
    assert_eq!(cleared.late_until, Some(None));
    assert_eq!(cleared.max_attempts, Some(None));

    let set = update(json!({
        "due_at": "2030-01-01T00:00:00Z", "on_deadline": "keep_draft", "accept_late": true,
        "late_until": "2030-01-02T00:00:00Z", "max_attempts": 3
    }));
    assert!(matches!(set.due_at, Some(Some(_))));
    assert_eq!(set.on_deadline, Some(Some("keep_draft".to_owned())));
    assert_eq!(set.accept_late, Some(Some(true)));
    assert_eq!(set.max_attempts, Some(Some(3)));
}

#[test]
fn a_new_assignment_has_no_settings_unless_sent() {
    let bare = new_assignment(json!({"type": "text", "max_points": "5"}));
    assert_eq!(bare.due_at, None);
    assert_eq!(bare.on_deadline, None);
    assert_eq!(bare.accept_late, None);
    assert_eq!(bare.late_until, None);
    assert_eq!(bare.max_attempts, None);
    let set = new_assignment(json!({"on_deadline": "auto_submit", "max_attempts": 2}));
    assert_eq!(set.on_deadline.as_deref(), Some("auto_submit"));
    assert_eq!(set.max_attempts, Some(2));
}

#[test]
fn a_tree_summary_without_an_assignment_has_no_key() {
    let summary = ItemSummary {
        id: Uuid::nil(),
        kind: ItemKind::Text,
        title: "t".into(),
        status: ContentStatus::Draft,
        publish_at: None,
        position: 1,
        assignment: None,
    };
    assert!(
        serde_json::to_value(summary)
            .unwrap()
            .get("assignment")
            .is_none()
    );
}
