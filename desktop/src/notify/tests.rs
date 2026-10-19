use serde_json::json;

use super::*;

#[test]
fn a_server_time_becomes_unix_seconds() {
    assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(parse_rfc3339("2023-11-14T22:13:20Z"), Some(1_700_000_000));
    assert_eq!(parse_rfc3339("2000-02-29T00:00:00Z"), Some(951_782_400));
    assert_eq!(
        parse_rfc3339("2023-11-14T22:13:20.123456Z"),
        Some(1_700_000_000)
    );
    assert_eq!(
        parse_rfc3339("2023-11-15T01:13:20+03:00"),
        Some(1_700_000_000)
    );
    assert_eq!(
        parse_rfc3339("2023-11-14T17:13:20-05:00"),
        Some(1_700_000_000)
    );
    for bad in ["", "2023-11-14", "tomorrow", "2023-11-14T22:13:20"] {
        assert_eq!(parse_rfc3339(bad), None, "{bad}");
    }
}

#[test]
fn it_round_trips_with_the_writer_in_core() {
    for ms in [
        0,
        951_782_400_000,
        1_700_000_000_000,
        1_709_164_799_000,
        4_102_444_800_000,
    ] {
        assert_eq!(
            parse_rfc3339(&crate::core::rfc3339(ms)),
            Some(ms / 1000),
            "{ms}"
        );
    }
}

fn work(due: &str, status: &str) -> serde_json::Value {
    json!({"courses": [], "items": [{"assignment_id": "a1", "course_id": "c1", "title": "Essay", "type": "text", "status": status, "due_at": due, "max_points": "10"}]})
}

#[test]
fn work_is_announced_a_day_and_an_hour_ahead() {
    let now = parse_rfc3339("2026-10-30T12:00:00Z").unwrap();
    let windows = |due: &str, status: &str| {
        due_notices(&work(due, status), now)
            .into_iter()
            .map(|n| n.window)
            .collect::<Vec<_>>()
    };
    assert_eq!(windows("2026-10-31T11:00:00Z", "not_started"), ["day"]);
    assert_eq!(windows("2026-10-30T12:30:00Z", "draft"), ["hour"]);
    assert_eq!(
        windows("2026-10-30T13:00:00Z", "draft"),
        ["hour"],
        "exactly an hour is the hour"
    );
    assert!(
        windows("2026-11-02T12:00:00Z", "not_started").is_empty(),
        "too far ahead"
    );
    assert!(
        windows("2026-10-30T11:59:00Z", "not_started").is_empty(),
        "already past"
    );
    assert!(
        windows("2026-10-30T12:30:00Z", "submitted").is_empty(),
        "turned in"
    );
    assert!(windows("2026-10-30T12:30:00Z", "graded").is_empty());
}

#[test]
fn a_work_list_without_the_fields_gives_nothing() {
    let now = 0;
    assert!(due_notices(&json!({}), now).is_empty());
    assert!(due_notices(&json!({"items": [{"status": "draft"}]}), now).is_empty());
    assert!(due_notices(&json!({"items": [{"status": "draft", "due_at": null, "assignment_id": "a", "title": "t"}]}), now).is_empty());
}

#[test]
fn the_words_follow_the_language_of_the_window() {
    assert_eq!(words("en", "hour"), "Due within an hour");
    assert_eq!(words("uk", "hour"), "Дедлайн за годину");
    assert_eq!(words("uk", "day"), "Дедлайн протягом доби");
    assert_eq!(
        words("de", "day"),
        "Due within a day",
        "a language the app does not have gets English"
    );
}
