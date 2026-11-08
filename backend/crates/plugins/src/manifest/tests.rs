use serde_json::json;

use super::*;
use crate::settings::SettingScope;
use access::CourseRole;
use common::Role;

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
fn a_plugin_offers_one_page_in_a_slot() {
    let mut value = hello();
    value["ui"] = json!([
        {"slot": "assignment.tab", "title": "One", "path": "/one"},
        {"slot": "assignment.tab", "title": "Two", "path": "/two"}
    ]);
    assert!(message(value).contains("`assignment.tab` has two pages"));
}

#[test]
fn a_long_value_is_cut_in_the_message() {
    let mut value = hello();
    value["scopes"] = json!(["x".repeat(500)]);
    // The message names the value cut short, then the scopes there are: it does not grow with the value.
    let text = message(value);
    assert!(!text.contains(&"x".repeat(50)), "{text}");
    assert!(text.len() < 300, "{text}");
}

#[test]
fn the_settings_of_a_manifest_are_read_with_their_defaults_and_scopes() {
    let mut value = hello();
    value["settings"] = json!({
        "unit": {"type": "string", "enum": ["words", "characters"], "default": "words"},
        "max_words": {"type": "number", "minimum": 0, "scope": "course"},
        "api_token": {"type": "string", "secret": true, "description": "Its key"},
        "tags": {"type": "array", "items": {"type": "string"}}
    });
    let manifest = parse(value).unwrap();
    assert_eq!(manifest.settings.len(), 4);
    assert_eq!(manifest.settings["unit"].default, Some(json!("words")));
    assert_eq!(manifest.settings["max_words"].scope, SettingScope::Course);
    assert!(manifest.settings["api_token"].secret);
    assert!(parse(hello()).unwrap().settings.is_empty());
}

#[test]
fn a_setting_that_is_not_valid_stops_the_install_and_is_named() {
    for (settings, part) in [
        (json!({"Bad-Key": {"type": "string"}}), "`Bad-Key`"),
        (json!({"a": {"type": "object"}}), "setting `a` is not valid"),
        (json!({"a": {"type": "string", "pattern": "x"}}), "pattern"),
        (
            json!({"a": {"type": "string", "secret": true, "default": "x"}}),
            "no default",
        ),
        (json!({"a": "string"}), "setting `a` is not valid"),
    ] {
        let mut value = hello();
        value["settings"] = settings.clone();
        let text = message(value);
        assert!(text.contains(part), "{settings}: {text}");
    }
    let mut many = hello();
    many["settings"] = (0..41)
        .map(|n| (format!("s{n}"), json!({"type": "boolean"})))
        .collect::<serde_json::Map<_, _>>()
        .into();
    assert!(message(many).contains("more than 40"));
}

fn with_routes(routes: serde_json::Value) -> serde_json::Value {
    let mut value = hello();
    value["routes"] = routes;
    value
}

#[test]
fn the_routes_of_a_manifest_are_read_with_their_roles() {
    let manifest = parse(with_routes(json!([
        {"method": "get", "path": "/api/summary/{course_id}",
         "roles": ["admin", "teacher", "teacher"], "course_roles": ["teacher", "assistant"]},
        {"method": "POST", "path": "/api/echo", "roles": ["student"]},
    ])))
    .unwrap();
    assert_eq!(manifest.routes.len(), 2);
    let first = &manifest.routes[0];
    assert_eq!(first.method, "GET", "the method is upper case");
    assert_eq!(first.roles, [Role::Admin, Role::Teacher], "once each");
    assert_eq!(
        first.course_roles,
        [CourseRole::Teacher, CourseRole::Assistant]
    );
    assert!(manifest.routes[1].course_roles.is_empty());
    assert!(parse(hello()).unwrap().routes.is_empty());
    // Without `auth` a route is for people who are signed in.
    assert!(manifest.routes.iter().all(|r| r.auth == RouteAuth::Session));
}

#[test]
fn a_route_may_be_public_and_then_it_is_get_only_with_no_roles_and_no_course() {
    let manifest = parse(with_routes(json!([
        {"method": "get", "path": "/calendar/{feed}", "auth": "none"},
        {"method": "GET", "path": "/s", "auth": "session", "roles": ["admin"]},
    ])))
    .unwrap();
    let public = &manifest.routes[0];
    assert_eq!(public.auth, RouteAuth::None);
    assert!(public.roles.is_empty() && public.course_roles.is_empty());
    assert_eq!(manifest.routes[1].auth, RouteAuth::Session);
    for bad in [
        json!({"method": "GET", "path": "/a", "auth": "anyone"}),
        json!({"method": "GET", "path": "/a", "auth": "none", "roles": ["admin"]}),
        json!({"method": "GET", "path": "/a", "auth": "none", "course_roles": ["teacher"]}),
        json!({"method": "GET", "path": "/a/{course_id}", "auth": "none"}),
        json!({"method": "POST", "path": "/a", "auth": "none"}),
        json!({"method": "DELETE", "path": "/a", "auth": "none"}),
        // A session route still needs its roles.
        json!({"method": "GET", "path": "/a", "auth": "session"}),
    ] {
        assert!(parse(with_routes(json!([bad.clone()]))).is_err(), "{bad}");
    }
}

#[test]
fn a_route_that_is_not_safe_or_not_clear_stops_the_install() {
    let route = |method: &str,
                 path: &str,
                 roles: serde_json::Value,
                 course_roles: serde_json::Value| {
        json!([{"method": method, "path": path, "roles": roles, "course_roles": course_roles}])
    };
    let bad = [
        route("TRACE", "/a", json!(["admin"]), json!([])),
        route("GET", "a", json!(["admin"]), json!([])),
        route("GET", "/", json!(["admin"]), json!([])),
        route("GET", "/a/", json!(["admin"]), json!([])),
        route("GET", "/a//b", json!(["admin"]), json!([])),
        route("GET", "/a/../b", json!(["admin"]), json!([])),
        route("GET", "/a/./b", json!(["admin"]), json!([])),
        route("GET", "/a%2fb", json!(["admin"]), json!([])),
        route("GET", "/a?x=1", json!(["admin"]), json!([])),
        route("GET", "/a b", json!(["admin"]), json!([])),
        route("GET", "/{Course}", json!(["admin"]), json!([])),
        route("GET", "/{a}/{a}", json!(["admin"]), json!([])),
        route("GET", "/{}", json!(["admin"]), json!([])),
        route("GET", "/x{a}", json!(["admin"]), json!([])),
        route("GET", "/a", json!([]), json!([])),
        route("GET", "/a", json!(["owner"]), json!([])),
        route("GET", "/a", json!(["admin"]), json!(["teacher"])),
        route("GET", "/c/{course_id}", json!(["admin"]), json!(["boss"])),
        route(
            "GET",
            &format!("/{}", "a".repeat(201)),
            json!(["admin"]),
            json!([]),
        ),
    ];
    for routes in bad {
        assert!(parse(with_routes(routes.clone())).is_err(), "{routes}");
    }
    let twice = json!([
        {"method": "GET", "path": "/a", "roles": ["admin"]},
        {"method": "get", "path": "/a", "roles": ["teacher"]},
    ]);
    assert!(message(with_routes(twice)).contains("declared twice"));
    // The same path with another method is another route.
    assert!(
        parse(with_routes(json!([
            {"method": "GET", "path": "/a", "roles": ["admin"]},
            {"method": "POST", "path": "/a", "roles": ["admin"]},
        ])))
        .is_ok()
    );
    let many: Vec<_> = (0..21)
        .map(|n| json!({"method": "GET", "path": format!("/r{n}"), "roles": ["admin"]}))
        .collect();
    assert!(message(with_routes(json!(many))).contains("more than 20"));
}
