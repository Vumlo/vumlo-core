use super::*;

fn names(persona: Persona) -> Vec<&'static str> {
    specs(persona).iter().map(ToolSpec::name).collect()
}

#[test]
fn a_student_is_offered_only_lookups_of_their_own_data_and_staff_get_changes_to_propose() {
    assert_eq!(
        names(Persona::Student),
        [
            "my_work",
            "my_grades",
            "assignment_info",
            "save_draft",
            "submit_work"
        ]
    );
    let staff = [
        "teaching_overview",
        "course_gradebook",
        "course_content",
        "set_grade",
        "publish_grades",
        "set_deadline",
        "create_module",
        "create_text_item",
        "create_assignment",
        "create_quiz",
    ];
    assert_eq!(names(Persona::Teacher), staff);
    // An admin has the teacher's tools and, besides, the lookups and changes for people and the
    // organisation.
    let mut admin = staff.to_vec();
    admin.extend([
        "find_people",
        "org_overview",
        "create_term",
        "create_group",
        "add_group_members",
        "reset_password",
    ]);
    assert_eq!(names(Persona::Admin), admin);

    // A student gets only the two tools that change their own work, none of the staff's; a teacher
    // gets none of the student's. Every tool that changes something is on the list of write tools,
    // which never run from the loop.
    let own_work: Vec<_> = names(Persona::Student)
        .into_iter()
        .filter(|n| is_write(n))
        .collect();
    assert_eq!(own_work, ["save_draft", "submit_work"]);
    assert!(
        names(Persona::Teacher)
            .iter()
            .all(|n| { !["save_draft", "submit_work", "assignment_info", "my_work"].contains(n) })
    );
    let writes: Vec<_> = names(Persona::Teacher)
        .into_iter()
        .filter(|n| is_write(n))
        .collect();
    assert_eq!(
        writes,
        [
            "set_grade",
            "publish_grades",
            "set_deadline",
            "create_module",
            "create_text_item",
            "create_assignment",
            "create_quiz"
        ]
    );
    let admin_writes: Vec<_> = names(Persona::Admin)
        .into_iter()
        .filter(|n| is_write(n))
        .skip(writes.len())
        .collect();
    assert_eq!(
        admin_writes,
        [
            "create_term",
            "create_group",
            "add_group_members",
            "reset_password"
        ]
    );
}

#[test]
fn a_list_over_the_cap_is_cut_and_says_how_much_was_left_out() {
    let rows: Vec<u32> = (0..MAX_ROWS as u32 + 3).collect();
    let json = capped(&rows, |n| json!(n));
    let array = json.as_array().unwrap();
    assert_eq!(array.len(), MAX_ROWS + 1);
    assert_eq!(array[MAX_ROWS], json!({"note": "3 more rows not shown"}));
    assert_eq!(capped(&rows[..2], |n| json!(n)), json!([0, 1]));
}

#[test]
fn arguments_are_parsed_as_untrusted_text() {
    let id = Uuid::now_v7();
    let ok: CourseArg = args(&format!(r#"{{"course_id":"{id}"}}"#)).unwrap();
    assert_eq!(ok.course_id, id);
    for bad in [
        "",
        "not json",
        r#"{"course_id":"nope"}"#,
        r#"{"course":"x"}"#,
    ] {
        assert!(args::<CourseArg>(bad).is_err(), "{bad:?}");
    }
    assert!(args::<Value>("").is_ok(), "no arguments is an empty object");
}
