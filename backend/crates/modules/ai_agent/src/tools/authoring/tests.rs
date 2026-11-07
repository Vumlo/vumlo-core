use serde_json::json;

use super::*;

fn parse(name: &str, args: Value) -> Result<Write, String> {
    Write::parse(name, &args.to_string())
}

fn questions(value: Value) -> Vec<Question> {
    serde_json::from_value(value).unwrap()
}

#[test]
fn a_model_may_send_points_as_numbers_and_the_item_tools_name_their_course_and_module() {
    let (c, m) = (Uuid::now_v7(), Uuid::now_v7());
    let ok = parse(
        "create_assignment",
        json!({"course_id": c, "module_id": m, "title": "Essay", "max_points": 20}),
    )
    .unwrap();
    let Write::Assignment {
        max_points, text, ..
    } = ok
    else {
        panic!("{ok:?}")
    };
    assert_eq!((max_points.as_deref(), text), (Some("20"), None));

    let missing = parse(
        "create_assignment",
        json!({"course_id": c, "module_id": m, "title": "Essay"}),
    );
    assert!(missing.unwrap_err().contains("max_points"));
    assert!(
        parse(
            "create_text_item",
            json!({"course_id": c, "title": "Notes", "text": "x"})
        )
        .is_err(),
        "a module is required"
    );
    assert!(
        parse(
            "create_module",
            json!({"course_id": c, "title": "Unit 1", "published": true})
        )
        .is_err(),
        "no field that would publish"
    );
}

#[test]
fn a_document_is_paragraphs_split_on_blank_lines_and_nothing_else() {
    let doc = document("First line\ncontinues\n\n  Second  \n\n\n");
    assert_eq!(doc["v"], 1);
    let blocks = doc["blocks"].as_array().unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0]["id"], "p1");
    assert_eq!(blocks[0]["content"][0]["text"], "First line\ncontinues");
    assert_eq!(blocks[1]["content"][0]["text"], "Second");
    assert!(doc.get("attachments").is_none());
}

#[test]
fn a_quiz_becomes_the_cores_config_with_ids_for_options_and_a_total() {
    let q = questions(json!([
        {"prompt": "2 + 2?", "type": "single_choice", "points": 2, "options": ["3", "4", "5"], "correct": [1]},
        {"prompt": "Primes", "type": "multiple_choice", "points": "3", "options": ["2", "4", "5"], "correct": [2, 0]},
        {"prompt": "Half of 5?", "type": "number", "points": 1.5, "answer": 2.5, "tolerance": "0.01"},
        {"prompt": "Capital of France", "type": "short_text", "points": 1, "accepted": ["Paris", "paris "]},
        {"prompt": "Explain", "type": "long_text", "points": 2},
    ]));
    let (config, total) = quiz(&q).unwrap();
    assert_eq!(total.to_string(), "9.5");
    assert_eq!(config["v"], 1);
    let qs = config["questions"].as_array().unwrap();
    assert_eq!(qs.len(), 5);
    assert_eq!(
        (qs[0]["id"].as_str(), qs[0]["points"].as_str()),
        (Some("q1"), Some("2.00"))
    );
    assert_eq!(qs[0]["correct"], json!(["b"]));
    assert_eq!(qs[0]["options"][1]["id"], "b");
    assert_eq!(
        qs[0]["options"][1]["content"]["blocks"][0]["content"][0]["text"],
        "4"
    );
    assert_eq!(qs[1]["correct"], json!(["a", "c"]), "sorted, as letters");
    assert_eq!(
        (qs[2]["answer"].as_str(), qs[2]["tolerance"].as_str()),
        (Some("2.5"), Some("0.01"))
    );
    assert_eq!(qs[3]["accepted"], json!(["Paris", "paris"]));
    assert!(qs[4].get("options").is_none() && qs[4].get("correct").is_none());
}

#[test]
fn an_answer_key_that_does_not_fit_is_refused_with_the_question_named() {
    let bad = |q: Value| quiz(&questions(json!([q]))).unwrap_err();
    let choice = |kind: &str, options: Value, correct: Value| json!({"prompt": "?", "type": kind, "points": 1, "options": options, "correct": correct});
    assert!(
        bad(choice("single_choice", json!(["a", "b"]), json!([2]))).contains("question 1"),
        "an index that is no option"
    );
    assert!(bad(choice("single_choice", json!(["a", "b"]), json!([0, 1]))).contains("exactly one"));
    assert!(bad(choice("single_choice", json!(["a", "b"]), json!([]))).contains("correct"));
    assert!(bad(choice("multiple_choice", json!(["a", "b"]), json!([0, 0]))).contains("distinct"));
    assert!(bad(choice("single_choice", json!(["only"]), json!([0]))).contains("2 to 10"));
    assert!(bad(json!({"prompt": "n", "type": "number", "points": 1})).contains("answer"));
    assert!(
        bad(json!({"prompt": "s", "type": "short_text", "points": 1, "accepted": []}))
            .contains("accepted")
    );
    assert!(bad(json!({"prompt": " ", "type": "long_text", "points": 1})).contains("prompt"));
    assert!(bad(json!({"prompt": "x", "type": "long_text", "points": 0})).contains("points"));
    assert!(quiz(&[]).is_err());
    let many = questions(Value::Array(
        (0..31)
            .map(|_| json!({"prompt": "x", "type": "long_text", "points": 1}))
            .collect(),
    ));
    assert!(quiz(&many).unwrap_err().contains("1 to 30"));
}

#[test]
fn the_preview_marks_the_correct_answers_so_the_teacher_can_check_them() {
    let q = questions(json!([
        {"prompt": "2 + 2?", "type": "single_choice", "points": 2, "options": ["3", "4"], "correct": [1]},
        {"prompt": "Half of 5?", "type": "number", "points": 1, "answer": "2.5", "tolerance": "0.01"},
    ]));
    assert_eq!(preview(1, &q[0]), "1. 2 + 2? (2)\n   a) 3\n   b) 4 ✓");
    assert_eq!(preview(2, &q[1]), "2. Half of 5? (1)\n   = 2.5 ± 0.01");
    let long = "é".repeat(PREVIEW_CHARS + 5);
    assert_eq!(
        cut(&long).chars().count(),
        PREVIEW_CHARS + 1,
        "cut on a character, with an ellipsis"
    );
}

#[test]
fn what_is_stored_is_what_runs() {
    let (c, m) = (Uuid::now_v7(), Uuid::now_v7());
    let write = parse(
        "create_quiz",
        json!({
            "course_id": c, "module_id": m, "title": "Fractions",
            "questions": [{"prompt": "1/2 + 1/2?", "type": "number", "points": 1, "answer": 1}]
        }),
    )
    .unwrap();
    let again: Write = serde_json::from_str(&serde_json::to_string(&write).unwrap()).unwrap();
    assert_eq!(again.name(), "create_quiz");
    assert_eq!(
        NAMES,
        [
            "create_module",
            "create_text_item",
            "create_assignment",
            "create_quiz"
        ]
    );
}
