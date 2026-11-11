use super::*;

#[test]
fn a_kind_is_a_short_slug() {
    for good in ["a", "deadline_changed", "at-risk-digest", &"a".repeat(40)] {
        assert!(Kind::parse(good).is_ok(), "{good}");
    }
    for bad in ["", "A", "1a", "-a", "a b", "a.b", "é", &"a".repeat(41)] {
        assert!(Kind::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn a_subject_is_one_clean_line() {
    assert!(Subject::parse("Deadline moved").is_ok());
    assert!(Subject::parse("Дедлайн змінено").is_ok());
    assert!(Subject::parse(&"я".repeat(200)).is_ok());
    for bad in ["", "two\nlines", "bell\u{7}", &"x".repeat(201)] {
        assert!(Subject::parse(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn a_body_may_have_line_breaks_and_tabs_but_no_other_control() {
    assert!(Body::parse("line one\r\nline\ttwo\n").is_ok());
    assert!(Body::parse("").is_err());
    assert!(Body::parse("bad\u{0}byte").is_err());
    assert!(Body::parse("esc\u{1b}[0m").is_err());
    assert!(Body::parse(&"x".repeat(20_000)).is_ok());
    assert!(Body::parse(&"x".repeat(20_001)).is_err());
}

#[test]
fn a_link_path_is_a_plain_path() {
    for good in ["/", "/courses/1/items?x=1#a"] {
        assert!(LinkPath::parse(good).is_ok(), "{good}");
    }
    for bad in [
        "",
        "courses/1",
        "//evil.example/x",
        "/a//b",
        "/a\\b",
        "/a b",
        "/a\nb",
        "https://evil.example/",
        &format!("/{}", "a".repeat(500)),
    ] {
        assert!(LinkPath::parse(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn a_dedupe_key_is_printable_and_short() {
    assert!(DedupeKey::parse("assignment:7:2026-11-10T10:00").is_ok());
    assert!(DedupeKey::parse("").is_err());
    assert!(DedupeKey::parse("a\nb").is_err());
    assert!(DedupeKey::parse(&"k".repeat(101)).is_err());
}

#[test]
fn an_address_must_parse_and_the_debug_text_never_shows_it() {
    let address = Address::parse("maya@lms.test").unwrap();
    assert_eq!(address.as_str(), "maya@lms.test");
    assert!(!format!("{address:?}").contains("maya"));
    assert!(Address::parse("not an address").is_err());
    assert!(Address::parse("").is_err());
}

#[test]
fn a_state_round_trips_through_its_text() {
    for state in [
        MessageState::Pending,
        MessageState::Sent,
        MessageState::Dead,
        MessageState::Cancelled,
    ] {
        assert_eq!(state.as_str().parse::<MessageState>().unwrap(), state);
    }
    assert!("lost".parse::<MessageState>().is_err());
}
