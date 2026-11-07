use serde_json::json;

use super::*;

fn mine(messages: &[&str]) -> Vec<String> {
    messages.iter().map(|m| (*m).to_owned()).collect()
}

#[test]
fn a_draft_must_be_words_the_student_wrote_apart_from_whitespace() {
    let own = mine(&[
        "hello",
        "Here is my answer.\nIt has two lines   and odd  spacing.",
        "thanks",
    ]);
    assert!(quoted("Here is my answer.", &own), "a part of a message");
    assert!(
        quoted("It has two lines and odd spacing.", &own),
        "whitespace is no difference"
    );
    assert!(
        quoted(
            "  Here is my answer.\n\nIt has two lines and odd spacing.  ",
            &own
        ),
        "the whole message, padded"
    );

    // What the model wrote, changed, joined or invented is not theirs.
    assert!(
        !quoted("Here is my better answer.", &own),
        "an improved version"
    );
    assert!(
        !quoted("hello thanks", &own),
        "two messages stitched together"
    );
    assert!(
        !quoted("The mitochondria is the powerhouse of the cell.", &own),
        "invented"
    );
    assert!(
        !quoted("", &own) && !quoted("   \n ", &own),
        "nothing is not an answer"
    );
    assert!(!quoted("hello", &[]), "no words of theirs at all");
}

#[test]
fn only_a_saved_draft_needs_the_words_check_and_submitting_has_no_text_to_check() {
    let a = Uuid::now_v7();
    let save = Write::parse(
        "save_draft",
        &json!({"assignment_id": a, "text": "my words"}).to_string(),
    )
    .unwrap();
    assert!(save.check_words(&mine(&["my words"])).is_ok());
    assert!(
        save.check_words(&mine(&["other words"]))
            .unwrap_err()
            .contains("never write it for them")
    );
    let submit = Write::parse("submit_work", &json!({"assignment_id": a}).to_string()).unwrap();
    assert!(submit.check_words(&[]).is_ok());
}

#[test]
fn arguments_are_strict_and_the_tool_name_survives_storage() {
    let a = Uuid::now_v7();
    assert!(
        Write::parse("save_draft", &json!({"assignment_id": a}).to_string()).is_err(),
        "text is required"
    );
    assert!(
        Write::parse(
            "submit_work",
            &json!({"assignment_id": a, "text": "sneaked in"}).to_string()
        )
        .is_err()
    );
    assert!(Write::parse("submit_work", "[]").is_err());
    for (name, args) in [
        ("save_draft", json!({"assignment_id": a, "text": "x"})),
        ("submit_work", json!({"assignment_id": a})),
    ] {
        let write = Write::parse(name, &args.to_string()).unwrap();
        let again: Write = serde_json::from_str(&serde_json::to_string(&write).unwrap()).unwrap();
        assert_eq!(again.name(), name);
    }
    assert_eq!(NAMES, ["save_draft", "submit_work"]);
}

#[test]
fn a_student_document_has_a_paragraph_per_line_and_reads_back_as_the_same_text() {
    let doc = student_document("First line\n\n  Second line  \nThird");
    assert_eq!(doc["v"], 1);
    assert_eq!(doc["attachments"], json!([]));
    assert_eq!(doc["blocks"].as_array().unwrap().len(), 3);
    let map = doc.as_object().unwrap().clone();
    assert_eq!(text_of(&map), "First line\nSecond line\nThird");
    assert_eq!(
        text_of(&Map::new()),
        "",
        "a document with no blocks has no text"
    );
}
