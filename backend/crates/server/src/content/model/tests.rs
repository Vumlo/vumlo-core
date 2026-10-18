use super::*;

#[test]
fn the_status_of_work_follows_one_order_of_rules() {
    use WorkStatus::*;
    // (graded, takes_submissions, attempts, draft, closed)
    let cases = [
        ((true, true, 2, true, true), Graded),
        ((true, false, 0, false, false), Graded),
        ((false, false, 0, false, true), InClass),
        ((false, true, 1, true, true), Submitted),
        ((false, true, 0, true, true), Draft),
        ((false, true, 0, false, true), Missed),
        ((false, true, 0, false, false), NotStarted),
    ];
    for ((graded, takes, attempts, draft, closed), expected) in cases {
        assert_eq!(
            WorkStatus::of(graded, takes, attempts, draft, closed),
            expected,
            "{graded} {takes} {attempts} {draft} {closed}"
        );
    }
}

#[test]
fn a_revision_knows_what_changed_since_the_item_was_published() {
    let mut item = Item {
        id: Uuid::nil(),
        module_id: Uuid::nil(),
        course_id: Uuid::nil(),
        position: 1,
        kind: ItemKind::Text,
        title: "Lecture".to_owned(),
        body: Map::new(),
        status: ItemStatus::Draft,
        publish_at: None,
        assignment: None,
        published: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    assert!(
        !item.has_unpublished_changes(),
        "a draft has nothing to compare with"
    );
    item.published = Some(PublishedSlot {
        title: "Lecture".to_owned(),
        body: Map::new(),
        at: Utc::now(),
    });
    assert!(!item.has_unpublished_changes());
    item.title = "Lecture, revised".to_owned();
    assert!(item.has_unpublished_changes());
    let seen = item.clone().as_seen_by_students();
    assert_eq!(seen.title, "Lecture");
    assert!(seen.published.is_none());
    assert!(Revision::of(&item).has_unpublished_changes);
}

#[test]
fn title_is_trimmed_and_bounded() {
    assert_eq!(
        ContentTitle::parse("  Lectures ").unwrap().as_str(),
        "Lectures"
    );
    assert!(ContentTitle::parse("").is_err());
    assert!(ContentTitle::parse("   ").is_err());
    assert!(ContentTitle::parse("a\nb").is_err());
    assert!(ContentTitle::parse("a\u{7f}b").is_err());
    assert!(ContentTitle::parse(&"ł".repeat(200)).is_ok());
    assert!(ContentTitle::parse(&"ł".repeat(201)).is_err());
}

#[test]
fn enums_round_trip() {
    for status in [ModuleStatus::Draft, ModuleStatus::Published] {
        assert_eq!(status.as_str().parse::<ModuleStatus>(), Ok(status));
    }
    for status in [ItemStatus::Draft, ItemStatus::Published] {
        assert_eq!(status.as_str().parse::<ItemStatus>(), Ok(status));
    }
    assert_eq!("text".parse::<ItemKind>(), Ok(ItemKind::Text));
    assert!("lesson".parse::<ItemKind>().is_err());
    assert_eq!("assignment".parse::<ItemKind>(), Ok(ItemKind::Assignment));
    assert!("quiz".parse::<ItemKind>().is_err());
}

#[test]
fn max_points_accepts_decimal_strings_up_to_two_decimals() {
    for ok in ["20", "7.5", "20.00", "0.01", "9999.99", "9999", "0012"] {
        assert!(MaxPoints::parse(ok).is_ok(), "{ok}");
    }
}

#[test]
fn max_points_rejects_everything_else() {
    for bad in [
        "0",
        "0.00",
        "-1",
        "10000",
        "9999.991",
        "10000.00",
        "1.005",
        "abc",
        "",
        ".5",
        "5.",
        "+5",
        "1e2",
        " 5",
        "5 ",
        "1,5",
        "1.2.3",
        "99999999999999999999",
    ] {
        assert_eq!(MaxPoints::parse(bad), Err(ModelError::MaxPoints), "{bad:?}");
    }
}

#[test]
fn max_points_display_has_exactly_two_decimals() {
    for (raw, shown) in [
        ("20", "20.00"),
        ("7.5", "7.50"),
        ("20.00", "20.00"),
        ("0.01", "0.01"),
        ("9999.99", "9999.99"),
    ] {
        assert_eq!(MaxPoints::parse(raw).unwrap().to_string(), shown);
    }
}

#[test]
fn the_type_enum_parses_the_four_builtin_types_and_nothing_else() {
    for (text, kind) in [
        ("text", BuiltinType::Text),
        ("no_submission", BuiltinType::NoSubmission),
        ("quiz", BuiltinType::Quiz),
        ("form", BuiltinType::Form),
    ] {
        assert_eq!(BuiltinType::parse(text), Ok(kind));
        assert_eq!(kind.as_str(), text);
    }
    for bad in ["file", "", "Text", "Quiz", "No_Submission", "no-submission"] {
        assert_eq!(
            BuiltinType::parse(bad),
            Err(ModelError::AssignmentType),
            "{bad:?}"
        );
    }
    let message = ModelError::AssignmentType.to_string();
    for name in ["text", "no_submission", "quiz", "form"] {
        assert!(message.contains(name), "{message}");
    }
    assert!(!message.contains("file"));
}

#[test]
fn the_text_and_no_submission_types_take_an_empty_config_only() {
    let profiles = EffectiveProfiles::unrestricted();
    for assignment_type in [BuiltinType::Text, BuiltinType::NoSubmission] {
        let mut config = Map::new();
        assert_eq!(assignment_type.validate_config(&config, &profiles), Ok(()));
        config.insert("limit".to_owned(), Value::from(5));
        let error = assignment_type
            .validate_config(&config, &profiles)
            .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("limit"), "{message}");
        assert!(message.contains(assignment_type.as_str()), "{message}");
    }
}

fn at(day: u32) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&format!("2030-01-{day:02}T12:00:00Z"))
        .unwrap()
        .with_timezone(&Utc)
}

fn deadline_patch() -> SettingsPatch {
    SettingsPatch {
        due_at: Some(Some(at(10))),
        on_deadline: Some(Some("keep_draft".to_owned())),
        accept_late: Some(Some(false)),
        ..SettingsPatch::default()
    }
}

fn fields(result: Result<SubmissionSettings, Vec<SettingProblem>>) -> Vec<&'static str> {
    result
        .expect_err("the settings must be refused")
        .into_iter()
        .map(|problem| problem.field)
        .collect()
}

#[test]
fn a_deadline_comes_with_what_happens_at_it_and_whether_late_work_is_taken() {
    let set = SubmissionSettings::default()
        .resolve(&deadline_patch(), true)
        .unwrap();
    assert_eq!(set.due_at, Some(at(10)));
    assert_eq!(set.on_deadline, Some(OnDeadline::KeepDraft));
    assert_eq!(set.accept_late, Some(false));
    assert_eq!((set.late_until, set.max_attempts), (None, None));

    for (what, patch, expected) in [
        (
            "due_at alone",
            SettingsPatch {
                due_at: Some(Some(at(10))),
                ..SettingsPatch::default()
            },
            vec!["on_deadline", "accept_late"],
        ),
        (
            "on_deadline alone",
            SettingsPatch {
                on_deadline: Some(Some("auto_submit".to_owned())),
                ..SettingsPatch::default()
            },
            vec!["on_deadline"],
        ),
        (
            "accept_late alone",
            SettingsPatch {
                accept_late: Some(Some(true)),
                ..SettingsPatch::default()
            },
            vec!["accept_late"],
        ),
    ] {
        let result = SubmissionSettings::default().resolve(&patch, true);
        assert_eq!(fields(result), expected, "{what}");
    }
}

#[test]
fn late_until_needs_accept_late_and_a_later_moment_than_due_at() {
    let with_late = |accept: bool, late_until: DateTime<Utc>| SettingsPatch {
        accept_late: Some(Some(accept)),
        late_until: Some(Some(late_until)),
        ..deadline_patch()
    };
    let ok = SubmissionSettings::default().resolve(&with_late(true, at(11)), true);
    assert_eq!(ok.unwrap().late_until, Some(at(11)));
    for (what, patch) in [
        ("accept_late false", with_late(false, at(11))),
        ("equal to due_at", with_late(true, at(10))),
        ("before due_at", with_late(true, at(9))),
    ] {
        let result = SubmissionSettings::default().resolve(&patch, true);
        assert_eq!(fields(result), ["late_until"], "{what}");
    }
    let alone = SettingsPatch {
        late_until: Some(Some(at(11))),
        ..SettingsPatch::default()
    };
    assert_eq!(
        fields(SubmissionSettings::default().resolve(&alone, true)),
        ["late_until"]
    );
}

#[test]
fn max_attempts_is_a_whole_number_of_at_least_one() {
    let attempts = |value: i64| SettingsPatch {
        max_attempts: Some(Some(value)),
        ..SettingsPatch::default()
    };
    for good in [1, 2, i64::from(i32::MAX)] {
        let set = SubmissionSettings::default().resolve(&attempts(good), true);
        assert_eq!(
            set.unwrap().max_attempts,
            i32::try_from(good).ok(),
            "{good}"
        );
    }
    for bad in [0, -1, i64::from(i32::MAX) + 1] {
        let result = SubmissionSettings::default().resolve(&attempts(bad), true);
        assert_eq!(fields(result), ["max_attempts"], "{bad}");
    }
}

#[test]
fn a_bad_on_deadline_is_one_problem_on_its_field() {
    let patch = SettingsPatch {
        on_deadline: Some(Some("later".to_owned())),
        ..deadline_patch()
    };
    assert_eq!(
        fields(SubmissionSettings::default().resolve(&patch, true)),
        ["on_deadline"]
    );
}

#[test]
fn the_rules_are_judged_on_the_resulting_state() {
    let full = SubmissionSettings::default()
        .resolve(&deadline_patch(), true)
        .unwrap();
    // Absent fields change nothing.
    assert_eq!(full.resolve(&SettingsPatch::default(), true).unwrap(), full);
    // Clearing only the deadline leaves the other two without it.
    let only_due = SettingsPatch {
        due_at: Some(None),
        ..SettingsPatch::default()
    };
    assert_eq!(
        fields(full.resolve(&only_due, true)),
        ["on_deadline", "accept_late"]
    );
    // Clearing the three together removes the deadline.
    let all = SettingsPatch {
        due_at: Some(None),
        on_deadline: Some(None),
        accept_late: Some(None),
        ..SettingsPatch::default()
    };
    assert_eq!(
        full.resolve(&all, true).unwrap(),
        SubmissionSettings::default()
    );
    // A change of accept_late alone breaks an existing late_until.
    let late = full
        .resolve(
            &SettingsPatch {
                accept_late: Some(Some(true)),
                late_until: Some(Some(at(12))),
                ..SettingsPatch::default()
            },
            true,
        )
        .unwrap();
    let strict = SettingsPatch {
        accept_late: Some(Some(false)),
        ..SettingsPatch::default()
    };
    assert_eq!(fields(late.resolve(&strict, true)), ["late_until"]);
    // Moving due_at past an existing late_until does too.
    let later = SettingsPatch {
        due_at: Some(Some(at(20))),
        ..SettingsPatch::default()
    };
    assert_eq!(fields(late.resolve(&later, true)), ["late_until"]);
    // max_attempts clears with null and is independent of the deadline.
    let two = full
        .resolve(
            &SettingsPatch {
                max_attempts: Some(Some(2)),
                ..SettingsPatch::default()
            },
            true,
        )
        .unwrap();
    let unlimited = SettingsPatch {
        max_attempts: Some(None),
        ..SettingsPatch::default()
    };
    assert_eq!(two.resolve(&unlimited, true).unwrap().max_attempts, None);
}

#[test]
fn a_type_that_takes_no_submissions_refuses_every_value_but_accepts_null() {
    let everything = SettingsPatch {
        max_attempts: Some(Some(1)),
        ..deadline_patch()
    };
    assert_eq!(
        fields(SubmissionSettings::default().resolve(&everything, false)),
        ["due_at", "on_deadline", "accept_late", "max_attempts"]
    );
    let nulls = SettingsPatch {
        due_at: Some(None),
        on_deadline: Some(None),
        accept_late: Some(None),
        late_until: Some(None),
        max_attempts: Some(None),
    };
    assert_eq!(
        SubmissionSettings::default().resolve(&nulls, false),
        Ok(SubmissionSettings::default())
    );
    assert!(BuiltinType::Text.takes_submissions());
    assert!(!BuiltinType::NoSubmission.takes_submissions());
}

fn features(config: Value) -> Result<Option<Vec<Feature>>, ModelError> {
    editor_features(config.as_object().unwrap())
}

#[test]
fn the_editor_features_of_a_text_config_are_a_list_of_known_unique_names() {
    assert_eq!(features(serde_json::json!({})), Ok(None));
    assert_eq!(features(serde_json::json!({"editor": {}})), Ok(None));
    assert_eq!(
        features(serde_json::json!({"editor": {"features": ["code", "headings"]}})),
        Ok(Some(vec![Feature::Code, Feature::Headings]))
    );
    assert_eq!(
        features(serde_json::json!({"editor": {"features": []}})),
        Ok(Some(vec![]))
    );
    for (what, config) in [
        ("another key", serde_json::json!({"limit": 1})),
        ("editor not an object", serde_json::json!({"editor": []})),
        (
            "another editor key",
            serde_json::json!({"editor": {"a": 1}}),
        ),
        (
            "features not a list",
            serde_json::json!({"editor": {"features": "code"}}),
        ),
        (
            "unknown feature",
            serde_json::json!({"editor": {"features": ["code", "blink"]}}),
        ),
        (
            "not a string",
            serde_json::json!({"editor": {"features": [1]}}),
        ),
        (
            "twice",
            serde_json::json!({"editor": {"features": ["code", "code"]}}),
        ),
    ] {
        let error = features(config).expect_err(what);
        assert!(!error.to_string().contains("blink"), "{what}: {error}");
    }
    let message = features(serde_json::json!({"editor": {"features": ["code", "blink"]}}))
        .unwrap_err()
        .to_string();
    assert!(message.contains("features[1]"), "{message}");
    // The type validates through the same function.
    let config = serde_json::json!({"editor": {"features": ["blink"]}});
    assert!(
        BuiltinType::Text
            .validate_config(
                config.as_object().unwrap(),
                &EffectiveProfiles::unrestricted()
            )
            .is_err()
    );
    let config = serde_json::json!({"editor": {"features": ["code"]}});
    assert!(
        BuiltinType::Text
            .validate_config(
                config.as_object().unwrap(),
                &EffectiveProfiles::unrestricted()
            )
            .is_ok()
    );
    assert!(
        BuiltinType::NoSubmission
            .validate_config(
                config.as_object().unwrap(),
                &EffectiveProfiles::unrestricted()
            )
            .is_err()
    );
}

#[test]
fn empty_patches_are_recognised() {
    assert!(ModulePatch::default().is_empty());
    assert!(ItemPatch::default().is_empty());
    let sets_points = ItemPatch {
        max_points: Some(MaxPoints::parse("5").unwrap()),
        ..ItemPatch::default()
    };
    assert!(!sets_points.is_empty());
    assert!(sets_points.touches_assignment());
    let clears = ModulePatch {
        publish_at: Some(None),
        ..ModulePatch::default()
    };
    assert!(!clears.is_empty());
}
