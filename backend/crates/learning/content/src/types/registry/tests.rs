use std::collections::BTreeMap;

use activity::{
    ActivityType, Capabilities, CompletionPolicy, DocumentErrors, EffectiveProfiles, Stage,
    ValidContent,
};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::builtin_types;
use crate::types::{file, form, no_submission, quiz, text};

fn obj(value: &Value) -> &Map<String, Value> {
    value.as_object().unwrap()
}

fn profiles() -> EffectiveProfiles {
    EffectiveProfiles::unrestricted()
}

fn caps(submissions: bool, auto_graded: bool) -> Capabilities {
    Capabilities {
        submissions,
        auto_graded,
    }
}

#[test]
fn built_in_table() {
    let types = builtin_types().unwrap();
    let mut ids = types.available();
    ids.sort_unstable();
    assert_eq!(ids, ["file", "form", "no_submission", "quiz", "text"]);

    let table = [
        (
            text::ID,
            caps(true, false),
            CompletionPolicy::SubmittedOrGraded,
        ),
        (
            no_submission::ID,
            caps(false, false),
            CompletionPolicy::Graded,
        ),
        (
            quiz::ID,
            caps(true, true),
            CompletionPolicy::SubmittedOrGraded,
        ),
        (
            form::ID,
            caps(true, false),
            CompletionPolicy::SubmittedOrGraded,
        ),
        (
            file::ID,
            caps(true, false),
            CompletionPolicy::SubmittedOrGraded,
        ),
    ];
    for (id, capabilities, completion) in table {
        let kind = types.get(id).unwrap();
        assert_eq!(kind.id(), id);
        assert_eq!(kind.capabilities(), capabilities, "{id}");
        assert_eq!(kind.completion(), completion, "{id}");
    }
}

// The tests below ask each type through the contract; the rules themselves are tested in the
// type's own tests, against its functions.

fn submit(
    ty: &dyn ActivityType,
    config: &Value,
    content: &Value,
    stage: Stage,
) -> Result<ValidContent, DocumentErrors> {
    ty.validate_submission(obj(config), content, stage, &profiles())
}

fn doc(words: &str) -> Value {
    json!({
        "v": 1,
        "blocks": [{"id": "p", "type": "paragraph", "content": [{"type": "text", "text": words}]}],
        "attachments": []
    })
}

#[test]
fn text_answers_through_the_contract() {
    assert!(
        text::Text
            .validate_config(obj(&json!({"surprise": 1})), &profiles())
            .is_err()
    );
    let config = json!({});
    assert!(submit(&text::Text, &config, &doc("hello"), Stage::Submit).is_ok());
    assert!(submit(&text::Text, &config, &json!({"v": 2}), Stage::Submit).is_err());
}

#[test]
fn no_submission_answers_through_the_contract() {
    assert!(
        no_submission::NoSubmission
            .validate_config(obj(&json!({"anything": true})), &profiles())
            .is_err()
    );
    let refused = submit(
        &no_submission::NoSubmission,
        &json!({}),
        &doc("hello"),
        Stage::Submit,
    );
    assert!(refused.is_err());
}

fn quiz_config() -> Value {
    json!({
        "v": 1, "show_correct": true, "shuffle_questions": false,
        "questions": [
            {"id": "q1", "type": "single_choice", "points": "2", "prompt": doc("pick"),
             "options": [
                 {"id": "a", "content": doc("a")}, {"id": "b", "content": doc("b")}
             ],
             "correct": ["b"], "explanation": doc("because"), "grading_notes": "b"},
            {"id": "q2", "type": "long_text", "points": "3", "prompt": doc("explain")},
        ]
    })
}

#[test]
fn quiz_answers_through_the_contract() {
    assert!(
        quiz::Quiz
            .validate_config(obj(&json!({})), &profiles())
            .is_err()
    );
    assert_eq!(
        quiz::Quiz.points_total(obj(&quiz_config())),
        Some("5".parse().unwrap())
    );
    let config = quiz_config();
    let right = json!({"answers": {"q1": ["b"], "q2": doc("my words")}});
    assert!(submit(&quiz::Quiz, &config, &right, Stage::Submit).is_ok());
    let wrong = json!({"answers": {"q1": ["a"]}});
    assert!(submit(&quiz::Quiz, &config, &wrong, Stage::Submit).is_ok());
    let refused = json!({"answers": {"nope": 1}});
    assert!(submit(&quiz::Quiz, &config, &refused, Stage::Submit).is_err());
}

fn form_config() -> Value {
    json!({
        "v": 1, "note": "Fill it in",
        "fields": [
            {"id": "name", "type": "short", "label": "Name", "required": true,
             "rules": {"max_length": 10}},
            {"id": "topic", "type": "choice", "label": "Topic", "required": false,
             "options": [{"id": "x", "text": "X"}, {"id": "y", "text": "Y"}],
             "rules": {"exclusive": true}},
        ]
    })
}

#[test]
fn form_answers_through_the_contract() {
    assert!(
        form::Form
            .validate_config(obj(&json!({})), &profiles())
            .is_err()
    );
    let config = form_config();
    for (content, stage, accepted) in [
        (
            json!({"answers": {"name": "Ada", "topic": ["x"]}}),
            Stage::Submit,
            true,
        ),
        (json!({"answers": {}}), Stage::Draft, true),
        (json!({"answers": {}}), Stage::Submit, false),
    ] {
        let result = submit(&form::Form, &config, &content, stage);
        assert_eq!(result.is_ok(), accepted, "{content}");
    }
}

#[test]
fn file_answers_through_the_contract() {
    assert!(
        file::File
            .validate_config(obj(&json!({"max_files": 99})), &profiles())
            .is_err()
    );
    let config = json!({"max_files": 3, "accept": [".pdf"]});
    let one = Uuid::from_u128(1);
    let content = json!({"files": [one], "comment": doc("done")});
    assert!(submit(&file::File, &config, &content, Stage::Submit).is_ok());
    let refused = json!({"files": [one, one, one, one]});
    assert!(submit(&file::File, &config, &refused, Stage::Submit).is_err());

    // The names hook is the type's free function.
    let body = obj(&content);
    for name in ["essay.pdf", "essay.exe"] {
        let names = BTreeMap::from([(one, name.to_owned())]);
        assert_eq!(
            file::check_names(obj(&config), body, &names),
            file::File.check_file_names(obj(&config), body, &names),
            "{name}"
        );
    }
}

#[test]
fn only_a_file_type_checks_names() {
    let names = BTreeMap::new();
    let body = Map::new();
    for ty in [
        &text::Text as &dyn ActivityType,
        &no_submission::NoSubmission,
        &quiz::Quiz,
        &form::Form,
    ] {
        assert_eq!(ty.check_file_names(&body, &body, &names), Ok(()));
    }
}
