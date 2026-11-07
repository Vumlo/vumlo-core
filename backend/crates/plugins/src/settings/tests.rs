use serde_json::json;

use super::*;

fn spec(value: serde_json::Value) -> SettingSpec {
    serde_json::from_value(value).unwrap()
}

fn invalid(value: serde_json::Value) -> String {
    spec(value).validate("k").unwrap_err()
}

#[test]
fn a_coherent_spec_is_valid_and_defaults_to_an_institution_setting_in_plain_text() {
    let s = spec(json!({"type": "string", "default": "Hello", "description": "A greeting"}));
    assert_eq!(s.validate("greeting"), Ok(()));
    assert_eq!((s.scope, s.secret), (SettingScope::Institution, false));
    // What an install stores reads back as the same spec.
    let again: SettingSpec = serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
    assert_eq!(again, s);
}

#[test]
fn a_spec_that_contradicts_itself_is_refused_and_says_which_setting() {
    for (bad, part) in [
        (
            json!({"type": "number", "enum": ["a"]}),
            "`enum` is for strings",
        ),
        (json!({"type": "string", "minimum": 1}), "for numbers"),
        (
            json!({"type": "number", "minimum": 5, "maximum": 1}),
            "above",
        ),
        (
            json!({"type": "string", "items": {"type": "string"}}),
            "`items`",
        ),
        (json!({"type": "array"}), "`items`"),
        (json!({"type": "boolean", "secret": true}), "only a string"),
        (
            json!({"type": "string", "enum": ["a"], "secret": true}),
            "only a string",
        ),
        (
            json!({"type": "string", "secret": true, "default": "x"}),
            "no default",
        ),
        (json!({"type": "string", "enum": ["a", "a"]}), "distinct"),
        (json!({"type": "string", "enum": []}), "1 to 50"),
        (
            json!({"type": "string", "enum": ["a"], "default": "b"}),
            "default must be one of",
        ),
        (
            json!({"type": "number", "minimum": 1, "default": 0}),
            "default must be at least 1",
        ),
        (
            json!({"type": "string", "description": "two\nlines"}),
            "plain text",
        ),
    ] {
        let why = invalid(bad.clone());
        assert!(why.contains("`k`") && why.contains(part), "{bad}: {why}");
    }
}

#[test]
fn a_spec_with_a_field_this_core_does_not_enforce_is_refused() {
    // `pattern` would be silently unchecked, so it stops the install instead.
    let error = serde_json::from_value::<SettingSpec>(json!({"type": "string", "pattern": "^a"}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("pattern"), "{error}");
    assert!(serde_json::from_value::<SettingSpec>(json!({"type": "object"})).is_err());
    assert!(
        serde_json::from_value::<SettingSpec>(json!({"type": "string", "scope": "user"})).is_err()
    );
}

#[test]
fn a_value_must_be_of_the_kind_and_within_the_bounds_of_its_spec() {
    let text = spec(json!({"type": "string"}));
    assert_eq!(text.check(&json!("hi")), Ok(()));
    assert!(text.check(&json!(1)).is_err());
    assert!(text.check(&json!("a\u{0}b")).is_err());
    assert!(text.check(&json!("x".repeat(2001))).is_err());

    let number = spec(json!({"type": "number", "minimum": 1, "maximum": 10}));
    assert_eq!(number.check(&json!(5)), Ok(()));
    assert_eq!(number.check(&json!(2.5)), Ok(()));
    assert!(number.check(&json!(0)).unwrap_err().contains("at least 1"));
    assert!(number.check(&json!(11)).unwrap_err().contains("at most 10"));
    assert!(number.check(&json!("5")).is_err());

    let flag = spec(json!({"type": "boolean"}));
    assert_eq!(flag.check(&json!(false)), Ok(()));
    assert!(flag.check(&json!("true")).is_err());

    let choice = spec(json!({"type": "string", "enum": ["fast", "slow"]}));
    assert_eq!(choice.check(&json!("fast")), Ok(()));
    assert!(
        choice
            .check(&json!("medium"))
            .unwrap_err()
            .contains("fast, slow")
    );

    let strings = spec(json!({"type": "array", "items": {"type": "string"}}));
    assert_eq!(strings.check(&json!(["a", "b"])), Ok(()));
    assert_eq!(strings.check(&json!([])), Ok(()));
    assert!(strings.check(&json!(["a", 1])).is_err());
    assert!(strings.check(&json!("a")).is_err());
    assert!(strings.check(&json!(vec!["x"; 51])).is_err());
    let numbers = spec(json!({"type": "array", "items": {"type": "number"}}));
    assert_eq!(numbers.check(&json!([1, 2.5])), Ok(()));
    assert!(numbers.check(&json!(["1"])).is_err());
}

#[test]
fn a_secret_cannot_be_empty() {
    let secret = spec(json!({"type": "string", "secret": true}));
    assert_eq!(secret.validate("k"), Ok(()));
    assert_eq!(secret.check(&json!("s3cret")), Ok(()));
    assert!(secret.check(&json!("")).is_err());
}

#[test]
fn text_from_a_command_line_is_text_for_a_string_and_json_for_the_rest() {
    let text = spec(json!({"type": "string"}));
    assert_eq!(text.parse_text(" 12 "), Ok(json!(" 12 ")));
    assert_eq!(
        spec(json!({"type": "number"})).parse_text("3"),
        Ok(json!(3))
    );
    assert_eq!(
        spec(json!({"type": "boolean"})).parse_text("true"),
        Ok(json!(true))
    );
    let list = spec(json!({"type": "array", "items": {"type": "string"}}));
    assert_eq!(list.parse_text(r#"["a","b"]"#), Ok(json!(["a", "b"])));
    assert!(list.parse_text("a,b").is_err());
    assert!(spec(json!({"type": "number"})).parse_text("three").is_err());
}

#[test]
fn a_key_is_a_lowercase_identifier() {
    for good in ["a", "api_token", "max_words2"] {
        assert!(is_key(good), "{good}");
    }
    for bad in ["", "Api", "2fa", "a-b", "a b", &"a".repeat(41)] {
        assert!(!is_key(bad), "{bad}");
    }
}
