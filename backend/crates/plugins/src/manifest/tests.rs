use serde_json::json;

use super::*;

fn parse(value: serde_json::Value) -> Result<PluginManifest, PluginsError> {
    parse_manifest(value.to_string().as_bytes())
}

fn message(value: serde_json::Value) -> String {
    parse(value).unwrap_err().to_string()
}

fn hello() -> serde_json::Value {
    json!({
        "id": "hello",
        "version": "1.0.0",
        "api": "1",
        "subscribes": ["submission.submitted"],
        "scopes": ["submissions:read", "reports:write"],
        "ui": [{"slot": "assignment.tab", "title": "Words", "path": "/ui/words"}]
    })
}

#[test]
fn a_manifest_that_asks_for_what_is_allowed_is_read() {
    let manifest = parse(hello()).unwrap();
    assert_eq!(manifest.id, "hello");
    assert_eq!(manifest.subscribes, ["submission.submitted"]);
    assert_eq!(
        manifest.scopes,
        [Scope::SubmissionsRead, Scope::ReportsWrite]
    );
    assert_eq!(manifest.ui[0].path, "/ui/words");
}

#[test]
fn a_manifest_may_ask_for_nothing_and_may_say_more_than_the_core_reads() {
    let manifest =
        parse(json!({"id": "quiet", "version": "2", "api": "1", "name": "Quiet one"})).unwrap();
    assert!(manifest.subscribes.is_empty() && manifest.scopes.is_empty() && manifest.ui.is_empty());
}

#[test]
fn repeats_are_folded_into_one() {
    let mut value = hello();
    value["subscribes"] = json!(["grade.published", "grade.published"]);
    value["scopes"] = json!(["reports:write", "reports:write"]);
    let manifest = parse(value).unwrap();
    assert_eq!(manifest.subscribes, ["grade.published"]);
    assert_eq!(manifest.scopes, [Scope::ReportsWrite]);
}

#[test]
fn an_unknown_scope_event_or_slot_stops_the_install_and_is_named() {
    let mut value = hello();
    value["scopes"] = json!(["submissions:read", "users:read"]);
    let text = message(value);
    assert!(
        text.contains("`users:read`") && text.contains("submissions:read"),
        "{text}"
    );

    let mut value = hello();
    value["subscribes"] = json!(["user.password_reset_requested"]);
    let text = message(value);
    assert!(text.contains("user.password_reset_requested"), "{text}");
    assert!(text.contains("submission.submitted"), "{text}");

    let mut value = hello();
    value["ui"][0]["slot"] = json!("page.header");
    assert!(message(value).contains("`page.header`"));
}

#[test]
fn the_protocol_version_and_the_id_are_checked() {
    let mut value = hello();
    value["api"] = json!("2");
    assert!(message(value).contains("api `2` is not supported"));
    for id in [
        "",
        "Hello",
        "1hello",
        "hello world",
        "hello/../x",
        &"a".repeat(33),
    ] {
        let mut value = hello();
        value["id"] = json!(id);
        assert!(message(value).contains("id "), "{id}");
    }
    assert!(parse(json!({"id": "a", "api": "1"})).is_err(), "no version");
    assert!(parse_manifest(b"not json").is_err());
}

#[test]
fn a_ui_path_stays_on_the_plugins_own_origin() {
    for path in [
        "ui",
        "//evil.example/x",
        "/../etc",
        "/a b",
        "/a\\b",
        "https://evil.example",
        "/\u{0430}",
    ] {
        let mut value = hello();
        value["ui"][0]["path"] = json!(path);
        assert!(
            message(value).contains("must start with a single `/`"),
            "{path}"
        );
    }
    let mut value = hello();
    value["ui"][0]["title"] = json!("   ");
    assert!(message(value).contains("must be 1 to 60 characters"));
}

#[test]
fn a_long_value_is_cut_in_the_message() {
    let mut value = hello();
    value["scopes"] = json!(["x".repeat(500)]);
    assert!(message(value).len() < 200);
}
