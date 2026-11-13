use super::*;

#[test]
fn tokens_are_never_printed() {
    let token = SessionToken::generate();
    assert!(format!("{token:?}").contains("***"));
    assert!(!format!("{token:?}").contains(token.expose()));
}

#[test]
fn session_tokens_are_43_url_safe_chars_and_unique() {
    let a = SessionToken::generate();
    let b = SessionToken::generate();
    assert_eq!(a.expose().len(), 43);
    assert!(
        a.expose()
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    );
    assert_ne!(a, b);
    assert_eq!(SessionToken::from_presented(a.expose()), Some(a));
}

#[test]
fn malformed_presented_tokens_are_rejected() {
    let good = SessionToken::generate();
    for bad in [
        "",
        "short",
        &format!("{}=", good.expose()),
        &format!("{} ", &good.expose()[..42]),
        &good.expose().replace(&good.expose()[..1], "+"),
    ] {
        assert_eq!(SessionToken::from_presented(bad), None, "{bad:?}");
    }
}

#[test]
fn token_hash_is_sha256_of_the_token_text() {
    let token = SessionToken::from_presented(&"A".repeat(43)).unwrap();
    let expected: [u8; 32] = Sha256::digest("A".repeat(43).as_bytes()).into();
    assert_eq!(token.hash(), expected);
    assert_ne!(token.hash(), SessionToken::generate().hash());
}

#[test]
fn text_enums_round_trip() {
    for kind in [
        SessionKind::Web,
        SessionKind::App,
        SessionKind::Cli,
        SessionKind::Pat,
    ] {
        assert_eq!(kind.as_str().parse::<SessionKind>(), Ok(kind));
    }
}

#[test]
fn only_web_and_app_sessions_slide() {
    let slide = |kind: SessionKind| kind.lifetimes(Audience::App).slide;
    assert_eq!(slide(SessionKind::Web), Some(Duration::days(30)));
    assert_eq!(slide(SessionKind::App), Some(Duration::days(90)));
    assert_eq!(slide(SessionKind::Cli), None);
    assert_eq!(slide(SessionKind::Pat), None);
    for kind in [
        SessionKind::Web,
        SessionKind::App,
        SessionKind::Cli,
        SessionKind::Pat,
    ] {
        let lifetimes = kind.lifetimes(Audience::App);
        assert_eq!(lifetimes.cap, None);
        assert_eq!(lifetimes.refresh_gap, Duration::hours(1));
    }
}

#[test]
fn a_console_session_is_short_capped_and_refreshed_often() {
    let lifetimes = SessionKind::Web.lifetimes(Audience::Console);
    assert_eq!(lifetimes.initial, Duration::minutes(30));
    assert_eq!(lifetimes.slide, Some(Duration::minutes(30)));
    assert_eq!(lifetimes.cap, Some(Duration::hours(8)));
    assert_eq!(lifetimes.refresh_gap, Duration::minutes(1));
}
