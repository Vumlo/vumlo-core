use super::*;

#[test]
fn healthy_response_is_accepted() {
    let body = br#"{"status":"ok","database":"ok","version":"0.1.0","modules":[]}"#;
    let health = parse_health(200, body).unwrap();
    assert_eq!(health.version, "0.1.0");
}

#[test]
fn degraded_body_is_rejected() {
    let body = br#"{"status":"ok","database":"down","version":"0.1.0","modules":[]}"#;
    let e = parse_health(200, body).unwrap_err();
    assert!(e.to_string().contains("database=down"), "{e}");
}

#[test]
fn problem_details_are_reported() {
    let body = br#"{"type":"about:blank","title":"Service Unavailable","status":503,"detail":"database unavailable"}"#;
    let e = parse_health(503, body).unwrap_err();
    assert!(e.to_string().contains("database unavailable"), "{e}");
}

#[test]
fn unexpected_status_without_problem_body_is_rejected() {
    let e = parse_health(502, b"<html>bad gateway</html>").unwrap_err();
    assert!(e.to_string().contains("502"), "{e}");
}

#[test]
fn garbage_body_is_rejected() {
    assert!(parse_health(200, b"nope").is_err());
}
