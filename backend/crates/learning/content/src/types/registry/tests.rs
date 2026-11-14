use std::collections::BTreeMap;

use activity::{
    ActivityType, Capabilities, CompletionPolicy, EffectiveProfiles, Stage, ValidContent,
};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::builtin_types;
use crate::model::{AssignmentType, BuiltinType};
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

#[test]
fn the_ids_are_the_stored_names_of_the_old_enum() {
    for (id, old) in [
        (text::ID, BuiltinType::Text),
        (no_submission::ID, BuiltinType::NoSubmission),
        (quiz::ID, BuiltinType::Quiz),
        (form::ID, BuiltinType::Form),
        (file::ID, BuiltinType::File),
    ] {
        assert_eq!(id, old.as_str());
    }
}

// The parity tests below prove the new impls equal the old `AssignmentType for BuiltinType`. Task
// 12 deletes that trait, and these tests with it.

/// Everything of the contract that needs no submission: both sides must agree on it.
fn same_config_side(old: BuiltinType, new: &dyn ActivityType, config: &Value) {
    let config = obj(config);
    assert_eq!(
        old.validate_config(config, &profiles())
            .map_err(|e| e.to_string()),
        new.validate_config(config, &profiles())
            .map_err(|e| e.to_string()),
    );
    assert_eq!(old.points_total(config), new.points_total(config));
    assert_eq!(old.structure(config), new.structure(config));
    assert_eq!(old.student_config(config), new.student_config(config));
    let (mut a, mut b) = (config.clone(), config.clone());
    old.strip_computed(&mut a);
    new.strip_computed(&mut b);
    assert_eq!(a, b);
    assert_eq!(
        old.config_file_refs(config).map_err(|e| e.to_string()),
        new.config_file_refs(config).map_err(|e| e.to_string()),
    );
    assert_eq!(old.takes_submissions(), new.capabilities().submissions);
}

/// The submission side: validation, and for an accepted content what is read from it.
fn same_submission_side(
    old: BuiltinType,
    new: &dyn ActivityType,
    config: &Value,
    content: &Value,
    stage: Stage,
) -> Result<ValidContent, ()> {
    let config = obj(config);
    let old_result = old.validate_submission(config, content, stage, &profiles());
    let new_result = new.validate_submission(config, content, stage, &profiles());
    assert_eq!(old_result, new_result);
    let valid = old_result.map_err(|_| ())?;
    let body = valid.as_body();
    assert_eq!(old.auto_grade(config, body), new.auto_grade(config, body));
    assert_eq!(old.claims(config, body), new.claims(config, body));
    assert_eq!(
        old.submission_file_refs(body),
        new.submission_file_refs(config, body)
    );
    Ok(valid)
}

fn doc(words: &str) -> Value {
    json!({
        "v": 1,
        "blocks": [{"id": "p", "type": "paragraph", "content": [{"type": "text", "text": words}]}],
        "attachments": []
    })
}

#[test]
fn text_matches_the_old_type() {
    for config in [
        json!({}),
        json!({"editor": {"features": ["code"]}}),
        json!({"editor": {"features": ["no_such_feature"]}}),
        json!({"editor": {"features": ["code", "code"]}}),
        json!({"surprise": 1}),
    ] {
        same_config_side(BuiltinType::Text, &text::Text, &config);
    }
    assert!(
        text::Text
            .validate_config(obj(&json!({"surprise": 1})), &profiles())
            .is_err()
    );
    let config = json!({});
    let accepted = same_submission_side(
        BuiltinType::Text,
        &text::Text,
        &config,
        &doc("hello"),
        Stage::Submit,
    );
    assert!(accepted.is_ok());
    let refused = same_submission_side(
        BuiltinType::Text,
        &text::Text,
        &config,
        &json!({"v": 2}),
        Stage::Submit,
    );
    assert!(refused.is_err());
}

#[test]
fn no_submission_matches_the_old_type() {
    for config in [json!({}), json!({"anything": true})] {
        same_config_side(
            BuiltinType::NoSubmission,
            &no_submission::NoSubmission,
            &config,
        );
    }
    assert!(
        no_submission::NoSubmission
            .validate_config(obj(&json!({"anything": true})), &profiles())
            .is_err()
    );
    let refused = same_submission_side(
        BuiltinType::NoSubmission,
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
fn quiz_matches_the_old_type() {
    let mut wrong_shape = quiz_config();
    wrong_shape["surprise"] = json!(1);
    for config in [quiz_config(), wrong_shape, json!({})] {
        same_config_side(BuiltinType::Quiz, &quiz::Quiz, &config);
    }
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
    assert!(
        same_submission_side(
            BuiltinType::Quiz,
            &quiz::Quiz,
            &config,
            &right,
            Stage::Submit
        )
        .is_ok()
    );
    let wrong = json!({"answers": {"q1": ["a"]}});
    assert!(
        same_submission_side(
            BuiltinType::Quiz,
            &quiz::Quiz,
            &config,
            &wrong,
            Stage::Submit
        )
        .is_ok()
    );
    let refused = json!({"answers": {"nope": 1}});
    assert!(
        same_submission_side(
            BuiltinType::Quiz,
            &quiz::Quiz,
            &config,
            &refused,
            Stage::Submit
        )
        .is_err()
    );
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
fn form_matches_the_old_type() {
    let mut wrong_shape = form_config();
    wrong_shape["surprise"] = json!(1);
    for config in [form_config(), wrong_shape, json!({})] {
        same_config_side(BuiltinType::Form, &form::Form, &config);
    }
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
        let result = same_submission_side(BuiltinType::Form, &form::Form, &config, &content, stage);
        assert_eq!(result.is_ok(), accepted, "{content}");
    }
}

#[test]
fn file_matches_the_old_type() {
    for config in [
        json!({}),
        json!({"max_files": 3, "accept": [".pdf"]}),
        json!({"max_files": 99}),
        json!({"surprise": 1}),
    ] {
        same_config_side(BuiltinType::File, &file::File, &config);
    }
    assert!(
        file::File
            .validate_config(obj(&json!({"max_files": 99})), &profiles())
            .is_err()
    );
    let config = json!({"max_files": 3, "accept": [".pdf"]});
    let one = Uuid::from_u128(1);
    let content = json!({"files": [one], "comment": doc("done")});
    assert!(
        same_submission_side(
            BuiltinType::File,
            &file::File,
            &config,
            &content,
            Stage::Submit
        )
        .is_ok()
    );
    let refused = json!({"files": [one, one, one, one]});
    assert!(
        same_submission_side(
            BuiltinType::File,
            &file::File,
            &config,
            &refused,
            Stage::Submit
        )
        .is_err()
    );

    // The names hook is the old free function.
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
