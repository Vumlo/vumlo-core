use super::*;

#[test]
fn the_address_on_the_plugins_server_is_what_follows_its_id() {
    assert_eq!(
        tail_of("/plugins/hello/api/summary/x"),
        Some(("hello", "/api/summary/x".to_owned()))
    );
    assert_eq!(
        tail_of("/plugins/hello/a"),
        Some(("hello", "/a".to_owned()))
    );
    assert_eq!(tail_of("/plugins/hello"), None);
    assert_eq!(tail_of("/other/hello/a"), None);
}

#[test]
fn the_target_the_plugin_receives_keeps_the_path_of_its_base_address_and_the_query() {
    assert_eq!(target_of("http://127.0.0.1:9000", "/api/x", None), "/api/x");
    assert_eq!(
        target_of("http://127.0.0.1:9000", "/api/x", Some("a=1&b=2")),
        "/api/x?a=1&b=2"
    );
    assert_eq!(
        target_of("https://tools.example.edu/hello", "/api/x", Some("a=1")),
        "/hello/api/x?a=1"
    );
    assert_eq!(
        target_of("https://tools.example.edu/p/q", "/api/x", None),
        "/p/q/api/x"
    );
}

#[test]
fn a_media_type_is_compared_without_its_parameters_and_its_case() {
    assert_eq!(
        media_type("Application/JSON; charset=utf-8"),
        "application/json"
    );
    assert_eq!(media_type(" text/csv "), "text/csv");
    assert!(ANSWER_TYPES.contains(&media_type("text/csv; header=present").as_str()));
    for never in [
        "text/html",
        "application/xhtml+xml",
        "image/svg+xml",
        "text/javascript",
        "application/javascript",
        "application/xml",
        "text/xml",
        "multipart/x-mixed-replace",
    ] {
        assert!(
            !ANSWER_TYPES.contains(&media_type(never).as_str()),
            "{never}"
        );
    }
}
