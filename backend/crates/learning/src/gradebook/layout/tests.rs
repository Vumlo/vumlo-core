use super::*;

fn d(text: &str) -> Decimal {
    text.parse().expect("a decimal")
}

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

/// An assignment worth `max`, with a mark when `score` is given.
fn entry(n: u128, max: &str, score: Option<&str>, group: Option<Uuid>) -> Entry {
    Entry {
        assignment_id: id(n),
        max_points: d(max),
        is_bonus: false,
        group,
        score: score.map(d),
    }
}

fn bonus(mut e: Entry) -> Entry {
    e.is_bonus = true;
    e
}

fn custom() -> Settings {
    Settings {
        target: d("100"),
        pass_mark: Some(d("60")),
        layout: Layout::Custom,
    }
}

fn group_of(outcome: &Outcome, key: Uuid) -> &GroupResult {
    outcome
        .groups
        .iter()
        .find(|g| g.key == Some(key))
        .expect("the group has assignments")
}

// ---------------------------------------------------------------------------------------------
// The three real courses of the owner: the totals must be the ones Moodle showed.
// ---------------------------------------------------------------------------------------------

/// "Linear algebra 2025-26": 65.20 of 108 reachable, shown against a target of 100.
#[test]
fn linear_algebra_adds_up_to_the_total_the_teacher_sees() {
    let (seminars, colloquia, labs, project) = (id(1000), id(1001), id(1002), id(1003));
    let (bonus_group, midterms, quizzes) = (id(1004), id(1005), id(1006));
    let groups = vec![
        GroupDef {
            cap: Some(d("15")),
            ..GroupDef::plain(seminars, "Seminars")
        },
        GroupDef::plain(colloquia, "Colloquia"),
        GroupDef::plain(labs, "Labs"),
        GroupDef::plain(project, "Project"),
        GroupDef {
            bonus: true,
            ..GroupDef::plain(bonus_group, "Bonus")
        },
        GroupDef {
            best_k: Some(3),
            ..GroupDef::plain(midterms, "Midterms")
        },
        GroupDef {
            counts: false,
            ..GroupDef::plain(quizzes, "Quizzes")
        },
    ];
    let mut entries = Vec::new();
    let mut n = 0u128;
    let mut add = |entries: &mut Vec<Entry>, max: &str, score: Option<&str>, group: Option<Uuid>| {
        n += 1;
        entries.push(entry(n, max, score, group));
    };
    // 13 seminars worth 2 each: 26 reachable, capped at 15; seminar 13 is empty.
    for score in [
        Some("0.89"),
        Some("0.44"),
        Some("1.00"),
        Some("0.00"),
        Some("0.00"),
        Some("0.00"),
        Some("1.00"),
        Some("0.98"),
        Some("1.00"),
        Some("1.00"),
        Some("0.00"),
        Some("0.00"),
        None,
    ] {
        add(&mut entries, "2", score, Some(seminars));
    }
    add(&mut entries, "5", Some("4.00"), Some(colloquia));
    add(&mut entries, "5", Some("3.25"), Some(colloquia));
    add(&mut entries, "5", Some("3.90"), Some(labs));
    add(&mut entries, "4", Some("2.92"), Some(labs));
    add(&mut entries, "4", Some("3.80"), Some(labs));
    for (max, score) in [
        ("2", "2.00"),
        ("3", "2.25"),
        ("5", "3.55"),
        ("5", "4.72"),
        ("3", "2.43"),
        ("2", "1.98"),
        ("2", "0.00"),
    ] {
        add(&mut entries, max, Some(score), Some(project));
    }
    // 23 bonus problems worth 1 and an "error in a self-test" worth 2 that got 0.5.
    for _ in 0..23 {
        add(&mut entries, "1", None, Some(bonus_group));
    }
    add(&mut entries, "2", Some("0.50"), Some(bonus_group));
    // Four midterms worth 6: the best three count.
    for score in ["2.22", "1.29", "0.66", "1.08"] {
        add(&mut entries, "6", Some(score), Some(midterms));
    }
    // The final exam sits in no group; the quizzes are practice.
    add(&mut entries, "30", Some("19.00"), None);
    for _ in 0..13 {
        add(&mut entries, "1", Some("1.00"), Some(quizzes));
    }

    let outcome = compute(&custom(), &groups, &entries);

    assert_eq!(group_of(&outcome, seminars).total, d("6.31"));
    assert_eq!(group_of(&outcome, seminars).possible, d("15"));
    assert_eq!(group_of(&outcome, colloquia).total, d("7.25"));
    assert_eq!(group_of(&outcome, labs).total, d("10.62"));
    assert_eq!(group_of(&outcome, project).total, d("16.93"));
    assert_eq!(group_of(&outcome, project).possible, d("22"));
    assert_eq!(group_of(&outcome, bonus_group).total, d("0.50"));
    assert_eq!(group_of(&outcome, bonus_group).possible, d("0"));
    let midterm_result = group_of(&outcome, midterms);
    assert_eq!(midterm_result.total, d("4.59"));
    assert_eq!(midterm_result.possible, d("18"));
    // Ids run in the order they were added: the midterms are 50..53, and the lowest of them
    // (0.66, the third) is the one that does not count.
    assert_eq!(midterm_result.dropped, vec![id(52)]);
    assert_eq!(group_of(&outcome, quizzes).total, d("0"));
    assert_eq!(group_of(&outcome, quizzes).possible, d("0"));

    assert_eq!(outcome.total, d("65.20"));
    assert_eq!(outcome.possible, d("108"));
    assert_eq!(outcome.passed, Some(true));
}

/// "Modern C++": one graded task of five, everything else empty; the extra tasks are bonus.
#[test]
fn a_flat_course_shows_the_marks_against_the_target_not_against_what_is_graded() {
    let mut entries = vec![
        entry(1, "15", None, None),
        entry(2, "15", None, None),
        entry(3, "5", Some("2.75"), None),
        entry(4, "20", None, None),
        entry(5, "40", None, None),
    ];
    entries.push(bonus(entry(6, "10", None, None)));
    entries.push(bonus(entry(7, "100", None, None)));

    let outcome = compute(&custom(), &[], &entries);

    assert_eq!(outcome.total, d("2.75"));
    assert_eq!(outcome.possible, d("95"));
    assert_eq!(outcome.passed, Some(false));
    assert_eq!(outcome.groups.len(), 1);
    assert_eq!(outcome.groups[0].key, None);
}

/// "People and me": the five bonus points are earned, and are not part of the 100.
#[test]
fn bonus_points_add_to_the_total_and_not_to_what_is_possible() {
    let mut entries = vec![
        entry(1, "3", Some("2.5"), None),
        entry(2, "4", Some("3.5"), None),
        entry(3, "5", Some("4"), None),
        entry(4, "10", Some("5"), None),
        entry(5, "5", Some("4"), None),
        entry(6, "11", Some("9"), None),
        entry(7, "5", Some("4.5"), None),
        entry(8, "11", Some("10.5"), None),
        entry(9, "5", Some("5"), None),
        entry(10, "5", Some("5"), None),
        entry(11, "10", Some("7"), None),
        entry(12, "21", Some("21"), None),
        entry(13, "5", Some("0"), None),
    ];
    entries.push(bonus(entry(14, "5", Some("5"), None)));

    let outcome = compute(&custom(), &[], &entries);

    assert_eq!(outcome.total, d("86"));
    assert_eq!(outcome.possible, d("100"));
}

// ---------------------------------------------------------------------------------------------
// The rules, one at a time.
// ---------------------------------------------------------------------------------------------

#[test]
fn the_total_never_passes_the_target_but_the_raw_sum_is_kept() {
    let entries = vec![entry(1, "100", Some("100"), None), bonus(entry(2, "5", Some("5"), None))];
    let outcome = compute(&Settings::default(), &[], &entries);
    assert_eq!(outcome.total, d("100"));
    assert_eq!(outcome.raw_total, d("105"));
    assert_eq!(outcome.passed, None);
}

#[test]
fn best_k_counts_an_empty_mark_as_zero_and_breaks_ties_by_order() {
    let key = id(900);
    let groups = vec![GroupDef {
        best_k: Some(2),
        ..GroupDef::plain(key, "Homework")
    }];
    let entries = vec![
        entry(1, "5", Some("5"), Some(key)),
        entry(2, "5", None, Some(key)),
        entry(3, "5", Some("3"), Some(key)),
        entry(4, "5", None, Some(key)),
    ];
    let outcome = compute(&custom(), &groups, &entries);
    let group = group_of(&outcome, key);
    assert_eq!(group.total, d("8"));
    assert_eq!(group.counted, vec![id(1), id(3)]);
    assert_eq!(group.dropped, vec![id(2), id(4)]);
    assert_eq!(group.possible, d("10"));
}

#[test]
fn best_k_larger_than_the_group_counts_everything() {
    let key = id(900);
    let groups = vec![GroupDef {
        best_k: Some(5),
        ..GroupDef::plain(key, "Homework")
    }];
    let entries = vec![entry(1, "5", Some("5"), Some(key)), entry(2, "5", Some("1"), Some(key))];
    let outcome = compute(&custom(), &groups, &entries);
    assert_eq!(group_of(&outcome, key).total, d("6"));
    assert!(group_of(&outcome, key).dropped.is_empty());
}

#[test]
fn a_cap_cuts_the_sum_and_says_so() {
    let key = id(900);
    let groups = vec![GroupDef {
        cap: Some(d("5")),
        ..GroupDef::plain(key, "Extras")
    }];
    let entries = vec![
        entry(1, "4", Some("4"), Some(key)),
        entry(2, "4", Some("4"), Some(key)),
    ];
    let outcome = compute(&custom(), &groups, &entries);
    let group = group_of(&outcome, key);
    assert_eq!(group.total, d("5"));
    assert!(group.capped);
    assert_eq!(group.possible, d("5"));
}

#[test]
fn a_practice_group_counts_for_nothing_but_keeps_its_assignments() {
    let key = id(900);
    let groups = vec![GroupDef {
        counts: false,
        ..GroupDef::plain(key, "Quizzes")
    }];
    let entries = vec![entry(1, "1", Some("1"), Some(key)), entry(2, "10", Some("7"), None)];
    let outcome = compute(&custom(), &groups, &entries);
    assert_eq!(outcome.total, d("7"));
    assert_eq!(outcome.possible, d("10"));
    assert_eq!(group_of(&outcome, key).assignments, vec![id(1)]);
}

#[test]
fn a_bonus_group_adds_to_the_total_only() {
    let key = id(900);
    let groups = vec![GroupDef {
        bonus: true,
        ..GroupDef::plain(key, "Extra")
    }];
    let entries = vec![entry(1, "10", Some("4"), Some(key)), entry(2, "90", Some("50"), None)];
    let outcome = compute(&custom(), &groups, &entries);
    assert_eq!(outcome.total, d("54"));
    assert_eq!(outcome.possible, d("90"));
}

#[test]
fn an_assignment_in_an_unknown_group_is_loose() {
    let entries = vec![entry(1, "10", Some("4"), Some(id(404)))];
    let outcome = compute(&custom(), &[], &entries);
    assert_eq!(outcome.groups.len(), 1);
    assert_eq!(outcome.groups[0].key, None);
    assert_eq!(outcome.total, d("4"));
}

#[test]
fn a_group_without_assignments_is_not_in_the_result() {
    let groups = vec![GroupDef::plain(id(900), "Empty")];
    let outcome = compute(&custom(), &groups, &[entry(1, "10", None, None)]);
    assert_eq!(outcome.groups.len(), 1);
    assert_eq!(outcome.groups[0].key, None);
}

// ---------------------------------------------------------------------------------------------
// Setup status
// ---------------------------------------------------------------------------------------------

#[test]
fn setup_counts_the_unsorted_and_the_empty_and_checks_the_target() {
    let used = id(900);
    let empty_row = id(901);
    let groups = vec![
        GroupDef {
            row: Some(id(10)),
            ..GroupDef::plain(used, "Labs")
        },
        GroupDef {
            row: Some(empty_row),
            ..GroupDef::plain(id(902), "Nothing here")
        },
    ];
    let entries = vec![
        entry(1, "60", None, Some(used)),
        entry(2, "30", None, None),
        entry(3, "5", None, None),
    ];
    let status = setup(&custom(), &groups, &entries);
    assert_eq!(status.unsorted, 2);
    assert_eq!(status.empty_groups, vec![empty_row]);
    assert_eq!(status.possible, d("95"));
    assert!(!status.reaches_target);
}

#[test]
fn nothing_is_unsorted_when_a_group_stands_for_a_module() {
    let settings = Settings {
        layout: Layout::Course,
        ..Settings::default()
    };
    let entries = vec![entry(1, "100", None, None)];
    let status = setup(&settings, &[], &entries);
    assert_eq!(status.unsorted, 0);
    assert!(status.reaches_target);
}

// ---------------------------------------------------------------------------------------------
// Suggested groups
// ---------------------------------------------------------------------------------------------

#[test]
fn titles_with_a_common_stem_are_proposed_as_a_group() {
    let items: Vec<(Uuid, String)> = [
        "Lab 1: Transformation matrices",
        "Lab 2: Fourier transform",
        "Lab 3 - SVD",
        "Seminar 1",
        "Seminar 2",
        "Midterm exam 1",
        "Midterm exam 2",
        "Colloquium 2 2023-24",
        "Bonus: error in Self-test 1",
        "Bonus: Extra problem for Topic 7",
        "Project report",
        "Quiz 1",
        "Quiz 2",
    ]
    .iter()
    .enumerate()
    .map(|(i, title)| (id(i as u128 + 1), (*title).to_owned()))
    .collect();

    let found = suggest(&items);
    let titles: Vec<&str> = found.iter().map(|s| s.title.as_str()).collect();

    assert_eq!(titles, ["Lab", "Seminar", "Midterm exam", "Bonus", "Quiz"]);
    assert_eq!(found[0].assignments, vec![id(1), id(2), id(3)]);
}

#[test]
fn a_single_title_and_a_one_letter_stem_make_no_group() {
    let items = vec![
        (id(1), "A1".to_owned()),
        (id(2), "A2".to_owned()),
        (id(3), "Project".to_owned()),
    ];
    assert!(suggest(&items).is_empty());
}
