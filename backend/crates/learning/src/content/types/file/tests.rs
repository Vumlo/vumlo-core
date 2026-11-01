use serde_json::{Map, Value, json};

use super::*;
use crate::content::model::{AssignmentType, BuiltinType};

fn obj(value: &Value) -> &Map<String, Value> {
    value.as_object().unwrap()
}

fn profiles() -> EffectiveProfiles {
    EffectiveProfiles::unrestricted()
}

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn config_error(config: &Value) -> String {
    validate_config(obj(config)).unwrap_err().to_string()
}

fn check(config: &Value, content: Value, stage: Stage) -> Result<ValidContent, DocumentErrors> {
    validate_submission(obj(config), &content, stage, &profiles())
}

fn paths(config: &Value, content: Value, stage: Stage) -> Vec<String> {
    check(config, content, stage)
        .unwrap_err()
        .iter()
        .map(|error| error.path.clone())
        .collect()
}

fn files(count: u128) -> Value {
    Value::Array((1..=count).map(|n| json!(id(n))).collect())
}

#[test]
fn config_defaults_max_files_to_5() {
    validate_config(obj(&json!({}))).unwrap();
    // Six files are over the default; five are not.
    assert!(check(&json!({}), json!({"files": files(5)}), Stage::Draft).is_ok());
    assert_eq!(
        paths(&json!({}), json!({"files": files(6)}), Stage::Draft),
        ["files"]
    );
}

#[test]
fn config_rejects_max_files_0_and_21() {
    for bad in [json!(0), json!(21), json!(-1), json!("5"), json!(2.5)] {
        let message = config_error(&json!({"max_files": bad}));
        assert!(message.contains("max_files"), "{message}");
    }
    for ok in [1, 20] {
        validate_config(obj(&json!({"max_files": ok}))).unwrap();
    }
}

#[test]
fn config_rejects_accept_without_a_dot_or_uppercase() {
    for bad in [
        "pdf", ".PDF", ".Pdf", ".", ".P", ".pd f", ".p.df", "", ".пдф",
    ] {
        let message = config_error(&json!({"accept": [bad]}));
        assert!(message.contains("accept[0]"), "{bad:?}: {message}");
    }
    assert!(config_error(&json!({"accept": [".pdf", ".pdf"]})).contains("accept[1]"));
    let too_many: Vec<String> = (0..31).map(|n| format!(".e{n:02}")).collect();
    assert!(config_error(&json!({"accept": too_many})).contains("accept"));
    let long = format!(".{}", "a".repeat(17));
    assert!(config_error(&json!({"accept": [long]})).contains("accept[0]"));
    assert!(config_error(&json!({"accept": ".pdf"})).contains("accept"));
    let thirty: Vec<String> = (0..30).map(|n| format!(".e{n:02}")).collect();
    validate_config(obj(&json!({"accept": thirty}))).unwrap();
    validate_config(obj(
        &json!({"max_files": 3, "accept": [".pdf", ".docx", ".7z", ".c", ".h"]}),
    ))
    .unwrap();
}

#[test]
fn config_rejects_unknown_keys() {
    assert!(config_error(&json!({"max_file": 3})).contains("max_file"));
}

#[test]
fn the_config_is_shown_to_a_student_unchanged() {
    let config = json!({"max_files": 3, "accept": [".pdf"]});
    let file = BuiltinType::File;
    assert_eq!(
        Value::Object(file.student_config(obj(&config))),
        config,
        "it holds no answer key"
    );
    let mut stripped = obj(&config).clone();
    file.strip_computed(&mut stripped);
    assert_eq!(Value::Object(stripped), config);
    assert_eq!(file.points_total(obj(&config)), None);
    assert!(
        file.auto_grade(obj(&config), obj(&json!({"files": []})))
            .is_none()
    );
    assert!(file.takes_submissions());
}

#[test]
fn draft_over_max_files_is_rejected() {
    let config = json!({"max_files": 2});
    assert!(check(&config, json!({"files": files(2)}), Stage::Draft).is_ok());
    assert_eq!(
        paths(&config, json!({"files": files(3)}), Stage::Draft),
        ["files"]
    );
    assert_eq!(
        paths(&config, json!({"files": files(3)}), Stage::Submit),
        ["files"]
    );
}

#[test]
fn draft_with_a_file_outside_accept_is_rejected() {
    let config = json!({"accept": [".pdf", ".docx"]});
    let names = BTreeMap::from([
        (id(1), "essay.pdf".to_owned()),
        (id(2), "notes.txt".to_owned()),
        (id(3), "no-extension".to_owned()),
    ]);
    let content = |ids: &[u128]| {
        let ids: Vec<Uuid> = ids.iter().map(|n| id(*n)).collect();
        json!({"files": ids})
    };
    let body = |value: Value| obj(&value).clone();
    assert!(check_names(obj(&config), &body(content(&[1])), &names).is_ok());
    let errors = check_names(obj(&config), &body(content(&[1, 2, 3])), &names).unwrap_err();
    let found: Vec<_> = errors.iter().map(|error| error.path.as_str()).collect();
    assert_eq!(found, ["files[1]", "files[2]"]);
    // Without `accept` any name passes.
    assert!(check_names(obj(&json!({})), &body(content(&[2, 3])), &names).is_ok());
}

#[test]
fn accept_is_case_insensitive_for_the_file_name() {
    let accept = vec![".pdf".to_owned(), ".gz".to_owned()];
    assert!(accepts(&accept, "Report.PDF"));
    assert!(accepts(&accept, "report.pdf"));
    assert!(accepts(&accept, "my.report.Pdf"));
    // The last extension counts.
    assert!(!accepts(&accept, "report.pdf.exe"));
    assert!(!accepts(&accept, "report"));
    assert!(!accepts(&accept, "pdf"));
    assert!(!accepts(&accept, ".pdf.") && !accepts(&accept, "report."));
    // Nothing configured: anything goes.
    assert!(accepts(&[], "whatever.exe"));
}

#[test]
fn only_ascii_letters_are_folded_in_an_extension() {
    // U+212A KELVIN SIGN lower-cases to an ASCII `k`; it is not a `k` in a file name.
    let accept = vec![".kml".to_owned()];
    assert!(accepts(&accept, "map.kml"));
    assert!(accepts(&accept, "map.KML"));
    assert!(!accepts(&accept, "map.\u{212A}ml"));
}

#[test]
fn duplicate_file_ids_are_rejected() {
    let dup = json!({"files": [id(1), id(2), id(1)]});
    assert_eq!(paths(&json!({}), dup, Stage::Draft), ["files[2]"]);
}

#[test]
fn submit_needs_at_least_one_file() {
    let config = json!({});
    let empty = check(&config, json!({"files": []}), Stage::Submit).unwrap();
    assert!(empty.is_blank(), "the service refuses a blank submit");
    let missing = check(&config, json!({}), Stage::Submit).unwrap();
    assert!(missing.is_blank());
    // A comment alone is not a submission.
    let comment = json!({"v": 1, "blocks": [{"id": "p", "type": "paragraph",
        "content": [{"type": "text", "text": "hi"}]}], "attachments": []});
    assert!(
        check(&config, json!({"comment": comment}), Stage::Submit)
            .unwrap()
            .is_blank()
    );
    let one = check(&config, json!({"files": files(1)}), Stage::Submit).unwrap();
    assert!(!one.is_blank());
}

#[test]
fn draft_may_be_empty() {
    let valid = check(&json!({}), json!({"files": []}), Stage::Draft).unwrap();
    assert!(valid.is_blank());
    assert_eq!(valid.as_body().get("files"), Some(&json!([])));
    assert!(
        check(&json!({}), json!({}), Stage::Draft)
            .unwrap()
            .is_blank()
    );
}

#[test]
fn comment_uses_the_student_profile() {
    let paragraph = json!({"v": 1, "blocks": [{"id": "p", "type": "paragraph",
        "content": [{"type": "text", "text": "hi"}]}], "attachments": []});
    let valid = check(
        &json!({}),
        json!({"files": files(1), "comment": paragraph}),
        Stage::Submit,
    )
    .unwrap();
    assert!(valid.as_body().contains_key("comment"));
    // The student profile has no video.
    let video = json!({"v": 1, "blocks": [{"id": "v", "type": "video",
        "props": {"provider": "youtube", "video_id": "dQw4w9WgXcQ"}}], "attachments": []});
    assert_eq!(
        paths(
            &json!({}),
            json!({"files": files(1), "comment": video}),
            Stage::Draft
        ),
        ["comment.blocks[0]"]
    );
    // An empty comment is not stored.
    let blank = json!({"v": 1, "blocks": [], "attachments": []});
    let valid = check(
        &json!({}),
        json!({"files": files(1), "comment": blank}),
        Stage::Draft,
    )
    .unwrap();
    assert!(!valid.as_body().contains_key("comment"));
}

#[test]
fn the_shape_of_the_content_is_checked() {
    for bad in [
        json!([]),
        json!({"files": "x"}),
        json!({"files": ["not-a-uuid"]}),
        json!({"files": [5]}),
        json!({"files": [], "other": 1}),
    ] {
        assert!(
            check(&json!({}), bad.clone(), Stage::Draft).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn claims_list_the_file_ids() {
    let content = json!({"files": [id(2), id(1)]});
    let claims = claims(obj(&json!({})), obj(&content));
    assert_eq!(claims.files, [id(2), id(1)]);
    assert!(claims.people.is_empty() && claims.exclusive.is_empty());
    assert!(!claims.is_empty());
}
