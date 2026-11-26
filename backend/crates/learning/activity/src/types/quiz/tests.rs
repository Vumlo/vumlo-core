use serde_json::{Map, Value, json};

use super::*;

fn profiles() -> EffectiveProfiles {
    EffectiveProfiles::unrestricted()
}

fn doc(text: &str) -> Value {
    json!({
        "v": 1,
        "blocks": [{"id": "p", "type": "paragraph", "content": [{"type": "text", "text": text}]}],
        "attachments": []
    })
}

fn option(id: &str) -> Value {
    json!({"id": id, "content": doc(id)})
}

fn single(id: &str, points: &str) -> Value {
    json!({
        "id": id, "type": "single_choice", "points": points, "prompt": doc("pick"),
        "options": [option("a"), option("b"), option("c")], "correct": ["b"],
        "explanation": doc("because b"), "grading_notes": "b it is"
    })
}

fn multiple(id: &str, points: &str, partial: bool) -> Value {
    json!({
        "id": id, "type": "multiple_choice", "points": points, "prompt": doc("pick some"),
        "options": [option("a"), option("b"), option("c"), option("d")], "correct": ["a", "b"],
        "partial_credit": partial
    })
}

fn number(id: &str, points: &str) -> Value {
    json!({"id": id, "type": "number", "points": points, "prompt": doc("pi?"),
           "answer": "3.14", "tolerance": "0.01"})
}

fn short(id: &str, points: &str) -> Value {
    json!({"id": id, "type": "short_text", "points": points, "prompt": doc("capital?"),
           "accepted": ["Kyiv", " kiev "]})
}

fn long(id: &str, points: &str) -> Value {
    json!({"id": id, "type": "long_text", "points": points, "prompt": doc("explain")})
}

fn quiz(questions: Vec<Value>) -> Value {
    json!({"v": 1, "questions": questions, "shuffle_questions": true, "show_correct": true})
}

fn obj(value: &Value) -> &Map<String, Value> {
    value.as_object().unwrap()
}

fn message(config: &Value) -> String {
    validate_config(obj(config), &profiles())
        .unwrap_err()
        .to_string()
}

fn answers(value: Value) -> Value {
    json!({"answers": value})
}

fn graded(config: &Value, given: Value) -> AutoGrade {
    let valid =
        validate_submission(obj(config), &answers(given), Stage::Submit, &profiles()).unwrap();
    auto_grade(obj(config), valid.as_body()).unwrap()
}

#[test]
fn a_quiz_of_every_question_type_is_valid_and_its_points_add_up() {
    let config = quiz(vec![
        single("q1", "2"),
        multiple("q2", "3", true),
        number("q3", "1.5"),
        short("q4", "1"),
        long("q5", "2.5"),
    ]);
    assert_eq!(validate_config(obj(&config), &profiles()), Ok(()));
    assert_eq!(points_total(obj(&config)), Some("10".parse().unwrap()));
}

#[test]
fn a_config_with_the_wrong_shape_is_refused_with_a_message() {
    let mut config = quiz(vec![single("q1", "1")]);
    config["surprise"] = json!(1);
    assert!(
        message(&config).contains("wrong shape"),
        "{}",
        message(&config)
    );
    let mut config = quiz(vec![single("q1", "1")]);
    config["v"] = json!(2);
    assert!(message(&config).contains("`v` must be 1"));
    assert!(message(&quiz(vec![])).contains("1 to 100"));
}

#[test]
fn ids_and_points_are_checked() {
    for bad in ["", "a b", "é"] {
        let mut question = single("q1", "1");
        question["id"] = json!(bad);
        assert!(
            message(&quiz(vec![question])).contains("questions[0].id"),
            "{bad:?}"
        );
    }
    let duplicate = quiz(vec![single("q1", "1"), single("q1", "1")]);
    assert!(message(&duplicate).contains("questions[1].id"));
    for bad in ["0", "-1", "1.234", "x", "10000", ""] {
        let config = quiz(vec![single("q1", bad)]);
        assert!(message(&config).contains("questions[0].points"), "{bad:?}");
    }
}

#[test]
fn choice_questions_need_options_and_a_matching_key() {
    let mut question = single("q1", "1");
    question["correct"] = json!(["b", "c"]);
    assert!(message(&quiz(vec![question])).contains("exactly one"));
    let mut question = single("q1", "1");
    question["correct"] = json!(["z"]);
    assert!(message(&quiz(vec![question])).contains("questions[0].correct"));
    let mut question = multiple("q1", "1", false);
    question["correct"] = json!([]);
    assert!(message(&quiz(vec![question])).contains("at least one"));
    let mut question = multiple("q1", "1", false);
    question["correct"] = json!(["a", "a"]);
    assert!(message(&quiz(vec![question])).contains("once each"));
    let mut question = single("q1", "1");
    question["options"] = json!([option("a")]);
    assert!(message(&quiz(vec![question])).contains("2 to 10"));
    let mut question = single("q1", "1");
    question["options"] = json!([option("a"), option("a")]);
    assert!(message(&quiz(vec![question])).contains("options[1].id"));
}

#[test]
fn a_field_of_another_question_type_is_refused() {
    let mut question = number("q1", "1");
    question["options"] = json!([option("a"), option("b")]);
    assert!(message(&quiz(vec![question])).contains("`questions[0].options` does not belong"));
    let mut question = single("q1", "1");
    question["partial_credit"] = json!(true);
    assert!(message(&quiz(vec![question])).contains("partial_credit"));
    let mut question = long("q1", "1");
    question["accepted"] = json!(["x"]);
    assert!(message(&quiz(vec![question])).contains("accepted"));
}

#[test]
fn numbers_and_accepted_texts_are_checked() {
    let mut question = number("q1", "1");
    question["answer"] = json!("abc");
    assert!(message(&quiz(vec![question])).contains("questions[0].answer"));
    let mut question = number("q1", "1");
    question["tolerance"] = json!("-1");
    assert!(message(&quiz(vec![question])).contains("tolerance"));
    let mut question = short("q1", "1");
    question["accepted"] = json!([]);
    assert!(message(&quiz(vec![question])).contains("accepted"));
    let mut question = short("q1", "1");
    question["accepted"] = json!(["  "]);
    assert!(message(&quiz(vec![question])).contains("accepted"));
}

#[test]
fn a_prompt_that_is_not_a_document_names_where() {
    let mut question = single("q1", "1");
    question["prompt"] = json!("plain string");
    let text = message(&quiz(vec![question]));
    assert!(text.contains("questions[0].prompt"), "{text}");
    let mut question = single("q1", "1");
    question["options"][1]["content"] = json!({"nope": true});
    let text = message(&quiz(vec![question]));
    assert!(text.contains("questions[0].options[1].content"), "{text}");
}

#[test]
fn the_student_never_gets_the_key() {
    let config = quiz(vec![
        single("q1", "1"),
        multiple("q2", "1", true),
        number("q3", "1"),
        short("q4", "1"),
    ]);
    let seen = Value::Object(student_config(obj(&config))).to_string();
    for secret in [
        "\"correct\"",
        "\"answer\"",
        "tolerance",
        "accepted",
        "explanation",
        "partial_credit",
        "grading_notes",
        "because b",
        "kiev",
    ] {
        assert!(!seen.contains(secret), "{secret} in {seen}");
    }
    for kept in [
        "\"prompt\"",
        "\"options\"",
        "\"points\"",
        "shuffle_questions",
        "show_correct",
    ] {
        assert!(seen.contains(kept), "{kept} missing from {seen}");
    }
}

#[test]
fn answers_are_checked_and_stored_cleaned() {
    let config = quiz(vec![
        single("q1", "1"),
        multiple("q2", "1", false),
        number("q3", "1"),
        short("q4", "1"),
        long("q5", "1"),
    ]);
    let ok = validate_submission(
        obj(&config),
        &answers(json!({
            "q1": ["b"], "q2": [], "q3": " 3,14 ", "q4": "  Kyiv ", "q5": doc("because"),
        })),
        Stage::Submit,
        &profiles(),
    )
    .unwrap();
    assert!(!ok.is_blank());
    let stored = ok.as_body()["answers"].as_object().unwrap();
    assert_eq!(stored["q1"], json!(["b"]));
    assert!(!stored.contains_key("q2"), "an empty answer is not stored");
    assert_eq!(stored["q3"], json!("3,14"));
    assert_eq!(stored["q4"], json!("Kyiv"));
    assert!(stored["q5"].get("blocks").is_some());

    let blank = validate_submission(
        obj(&config),
        &answers(json!({"q1": []})),
        Stage::Draft,
        &profiles(),
    )
    .unwrap();
    assert!(blank.is_blank());
    let none = validate_submission(obj(&config), &json!({}), Stage::Draft, &profiles()).unwrap();
    assert!(none.is_blank());
}

#[test]
fn a_wrong_answer_names_its_question() {
    let config = quiz(vec![single("q1", "1"), number("q3", "1"), long("q5", "1")]);
    let cases = [
        (json!({"zz": ["a"]}), "answers.zz"),
        (json!({"q1": "b"}), "answers.q1"),
        (json!({"q1": ["z"]}), "answers.q1"),
        (json!({"q1": ["a", "b"]}), "answers.q1"),
        (json!({"q3": "abc"}), "answers.q3"),
        (json!({"q3": 3}), "answers.q3"),
        (json!({"q5": "text"}), "answers.q5"),
    ];
    for (given, path) in cases {
        let errors = validate_submission(
            obj(&config),
            &answers(given.clone()),
            Stage::Draft,
            &profiles(),
        )
        .unwrap_err();
        assert!(
            errors.iter().any(|error| error.path.starts_with(path)),
            "{given}: {errors:?}"
        );
    }
    for content in [json!([]), json!({"answers": []}), json!({"other": 1})] {
        assert!(
            validate_submission(obj(&config), &content, Stage::Draft, &profiles()).is_err(),
            "{content}"
        );
    }
}

#[test]
fn single_and_multiple_choice_score_all_or_nothing() {
    let config = quiz(vec![single("q1", "2"), multiple("q2", "3", false)]);
    let all = graded(&config, json!({"q1": ["b"], "q2": ["a", "b"]}));
    assert_eq!(all.total, "5".parse().unwrap());
    assert!(all.complete);
    assert_eq!(all.scores_json(), json!({"q1": "2.00", "q2": "3.00"}));
    let wrong = graded(&config, json!({"q1": ["a"], "q2": ["a"]}));
    assert_eq!(wrong.total, Decimal::ZERO);
    let extra = graded(&config, json!({"q2": ["a", "b", "c"]}));
    assert_eq!(extra.total, Decimal::ZERO);
}

#[test]
fn partial_credit_counts_right_minus_wrong_and_never_goes_below_zero() {
    let config = quiz(vec![multiple("q1", "3", true)]);
    let cases = [
        (json!(["a", "b"]), "3.00"),
        (json!(["a"]), "1.50"),
        (json!(["a", "c"]), "0.00"),
        (json!(["c", "d"]), "0.00"),
        (json!(["a", "b", "c"]), "1.50"),
        (json!([]), "0.00"),
    ];
    for (picked, expected) in cases {
        let grade = graded(&config, json!({"q1": picked.clone()}));
        assert_eq!(format!("{:.2}", grade.total), expected, "{picked}");
    }
    let odd = quiz(vec![json!({
        "id": "q1", "type": "multiple_choice", "points": "1", "prompt": doc("x"),
        "options": [option("a"), option("b"), option("c")], "correct": ["a", "b", "c"],
        "partial_credit": true
    })]);
    assert_eq!(
        format!("{:.2}", graded(&odd, json!({"q1": ["a"]})).total),
        "0.33"
    );
}

#[test]
fn numbers_are_scored_within_the_tolerance_and_text_ignores_case_and_spaces() {
    let config = quiz(vec![number("q1", "1"), short("q2", "1")]);
    for (given, expected) in [
        ("3.14", "1.00"),
        ("3,15", "1.00"),
        ("3.13", "1.00"),
        ("3.2", "0.00"),
        ("-3.14", "0.00"),
    ] {
        let grade = graded(&config, json!({"q1": given}));
        assert_eq!(format!("{:.2}", grade.scores["q1"]), expected, "{given}");
    }
    for (given, expected) in [("kyiv", "1.00"), (" KIEV ", "1.00"), ("Lviv", "0.00")] {
        let grade = graded(&config, json!({"q2": given}));
        assert_eq!(format!("{:.2}", grade.scores["q2"]), expected, "{given}");
    }
}

#[test]
fn unanswered_questions_score_zero_and_a_long_text_makes_the_quiz_incomplete() {
    let config = quiz(vec![single("q1", "1"), long("q2", "4")]);
    let grade = graded(&config, json!({"q2": doc("essay")}));
    assert_eq!(grade.scores.len(), 1);
    assert_eq!(grade.scores["q1"], Decimal::ZERO);
    assert!(!grade.complete);
    assert_eq!(grade.scores_json(), json!({"q1": "0.00"}));
}

#[test]
fn the_structure_ignores_wording_and_order_of_options_but_not_the_key() {
    let config = quiz(vec![single("q1", "1"), number("q2", "1")]);
    let mut reworded = config.clone();
    reworded["questions"][0]["prompt"] = doc("a different prompt");
    reworded["questions"][0]["explanation"] = doc("another");
    reworded["questions"][0]["grading_notes"] = json!("new notes");
    reworded["questions"][0]["options"][0]["content"] = doc("reworded");
    reworded["shuffle_questions"] = json!(false);
    reworded["show_correct"] = json!(false);
    assert_eq!(structure(obj(&config)), structure(obj(&reworded)));
    let mut reordered = config.clone();
    reordered["questions"][0]["options"] = json!([option("c"), option("b"), option("a")]);
    assert_eq!(structure(obj(&config)), structure(obj(&reordered)));

    let edits: [fn(&mut Value); 6] = [
        |c| c["questions"][0]["correct"] = json!(["a"]),
        |c| c["questions"][0]["points"] = json!("2"),
        |c| c["questions"][1]["answer"] = json!("3.15"),
        |c| c["questions"][1]["tolerance"] = json!("1"),
        |c| c["questions"][0]["options"] = json!([option("a"), option("b")]),
        |c| {
            let first = c["questions"][0].clone();
            c["questions"][0] = c["questions"][1].clone();
            c["questions"][1] = first;
        },
    ];
    for (n, edit) in edits.into_iter().enumerate() {
        let mut changed = config.clone();
        edit(&mut changed);
        assert_ne!(
            structure(obj(&config)),
            structure(obj(&changed)),
            "edit {n} should change the structure"
        );
    }
}

#[test]
fn the_result_shows_the_key_only_when_the_quiz_does() {
    let mut config = quiz(vec![single("q1", "2"), number("q2", "1"), short("q3", "1")]);
    let content = answers(json!({"q1": ["a"], "q2": "3.14"}));
    let scores = json!({"q1": "0.00", "q2": "1.00", "q3": "0.00"});

    let shown = result_questions(obj(&config), obj(&content), obj(&scores)).unwrap();
    assert_eq!(shown[0].correct, Some(json!(["b"])));
    assert_eq!(shown[0].answer, Some(json!(["a"])));
    assert_eq!(shown[0].got, Some(Decimal::ZERO));
    assert!(shown[0].explanation.is_some());
    assert_eq!(shown[1].correct, Some(json!("3.14")));
    assert_eq!(shown[2].answer, None);
    assert_eq!(shown[2].correct, Some(json!(["Kyiv", " kiev "])));

    config["show_correct"] = json!(false);
    for question in result_questions(obj(&config), obj(&content), obj(&scores)).unwrap() {
        assert_eq!(question.correct, None);
        assert_eq!(question.explanation, None);
    }
}
