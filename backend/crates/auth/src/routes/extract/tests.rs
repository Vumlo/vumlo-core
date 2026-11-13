use axum::http::HeaderValue;

use super::*;

fn server(allowed: &[&str]) -> ServerSettings {
    ServerSettings {
        bind_addr: ([127, 0, 0, 1], 0).into(),
        public_origin: "https://a.edu".to_owned(),
        admin_origin: "https://admin.a.edu".to_owned(),
        client_ip_header: None,
        allowed_origins: allowed.iter().map(|o| (*o).to_owned()).collect(),
    }
}

fn headers(origin: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(origin) = origin {
        headers.insert(header::ORIGIN, HeaderValue::from_str(origin).unwrap());
    }
    headers
}

/// The credential that is tried first.
fn credential(
    headers: &HeaderMap,
    server: &ServerSettings,
) -> Result<(Channel, SessionToken), AuthError> {
    candidates(headers, server).map(|mut found| found.remove(0))
}

const UNSAFE: [Method; 4] = [Method::POST, Method::PATCH, Method::DELETE, Method::PUT];

#[test]
fn read_only_methods_need_no_origin() {
    for audience in [Audience::App, Audience::Console] {
        for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
            assert!(check_origin(&method, &headers(None), &server(&[]), audience).is_ok());
        }
    }
}

#[test]
fn unsafe_methods_need_the_exact_origin() {
    for method in UNSAFE {
        assert!(
            check_origin(
                &method,
                &headers(Some("https://a.edu")),
                &server(&[]),
                Audience::App
            )
            .is_ok()
        );
        for bad in [
            None,
            Some("https://evil.test"),
            Some("https://a.edu.evil.test"),
            Some("null"),
            Some("https://a.edu/"),
        ] {
            assert!(
                matches!(
                    check_origin(
                        &method,
                        &headers(bad),
                        &server(&["https://app.b.edu"]),
                        Audience::App
                    ),
                    Err(AuthError::OriginMismatch)
                ),
                "{method} {bad:?}"
            );
        }
    }
}

#[test]
fn unsafe_methods_also_accept_an_allowed_origin_and_nothing_near_it() {
    let server = server(&["https://app.b.edu", "http://localhost:5173"]);
    for method in UNSAFE {
        for good in [
            "https://a.edu",
            "https://app.b.edu",
            "http://localhost:5173",
        ] {
            assert!(check_origin(&method, &headers(Some(good)), &server, Audience::App).is_ok());
        }
        for bad in [
            "https://b.edu",
            "https://app.b.edu.evil.test",
            "http://app.b.edu",
            "https://app.b.edu:8443",
            "https://APP.b.edu",
            "https://app.b.edu/",
            "*",
        ] {
            assert!(
                matches!(
                    check_origin(&method, &headers(Some(bad)), &server, Audience::App),
                    Err(AuthError::OriginMismatch)
                ),
                "{method} {bad}"
            );
        }
    }
}

#[test]
fn console_requests_need_exactly_the_admin_origin() {
    let server = server(&["https://app.b.edu"]);
    for method in UNSAFE {
        assert!(
            check_origin(
                &method,
                &headers(Some("https://admin.a.edu")),
                &server,
                Audience::Console
            )
            .is_ok()
        );
        for bad in [
            None,
            Some("https://a.edu"),
            Some("https://app.b.edu"),
            Some("https://admin.a.edu/"),
            Some("https://admin.a.edu.evil.test"),
            Some("null"),
        ] {
            assert!(
                matches!(
                    check_origin(&method, &headers(bad), &server, Audience::Console),
                    Err(AuthError::OriginMismatch)
                ),
                "{method} {bad:?}"
            );
        }
    }
}

#[test]
fn app_requests_no_longer_accept_the_admin_origin() {
    for method in UNSAFE {
        assert!(matches!(
            check_origin(
                &method,
                &headers(Some("https://admin.a.edu")),
                &server(&[]),
                Audience::App
            ),
            Err(AuthError::OriginMismatch)
        ));
    }
}

fn with_cookies(origin: Option<&str>, cookies: &[(&str, &SessionToken)]) -> HeaderMap {
    let mut h = headers(origin);
    let line = cookies
        .iter()
        .map(|(name, token)| format!("{name}={}", token.expose()))
        .collect::<Vec<_>>()
        .join("; ");
    h.insert(header::COOKIE, HeaderValue::from_str(&line).unwrap());
    h
}

#[test]
fn bearer_wins_over_both_cookies_and_junk_is_rejected() {
    let token = SessionToken::generate();
    let other = SessionToken::generate();
    let mut h = with_cookies(
        None,
        &[(COOKIE_NAME, &other), (CONSOLE_COOKIE_NAME, &other)],
    );
    h.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("bearer {}", token.expose())).unwrap(),
    );
    assert!(matches!(
        credential(&h, &server(&[])),
        Ok((Channel::Bearer, t)) if t == token
    ));

    for bad in ["Basic abc", "Bearer", "Bearer short", "token"] {
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, HeaderValue::from_static(bad));
        assert!(
            matches!(
                credential(&h, &server(&[])),
                Err(AuthError::Unauthenticated)
            ),
            "{bad}"
        );
    }
}

#[test]
fn the_session_cookie_is_read_and_missing_credentials_are_rejected() {
    let token = SessionToken::generate();
    let mut h = HeaderMap::new();
    h.insert(
        header::COOKIE,
        HeaderValue::from_str(&format!("theme=dark; {COOKIE_NAME}={}", token.expose())).unwrap(),
    );
    assert!(matches!(
        credential(&h, &server(&[])),
        Ok((Channel::SessionCookie, t)) if t == token
    ));
    assert!(matches!(
        credential(&HeaderMap::new(), &server(&[])),
        Err(AuthError::Unauthenticated)
    ));
}

#[test]
fn the_console_cookie_is_used_without_an_origin_or_with_the_admin_origin() {
    let console = SessionToken::generate();
    let app = SessionToken::generate();
    for origin in [None, Some("https://admin.a.edu")] {
        let h = with_cookies(
            origin,
            &[(COOKIE_NAME, &app), (CONSOLE_COOKIE_NAME, &console)],
        );
        assert!(
            matches!(
                credential(&h, &server(&[])),
                Ok((Channel::ConsoleCookie, t)) if t == console
            ),
            "{origin:?}"
        );
    }
    let alone = with_cookies(None, &[(CONSOLE_COOKIE_NAME, &console)]);
    assert!(matches!(
        credential(&alone, &server(&[])),
        Ok((Channel::ConsoleCookie, _))
    ));
}

#[test]
fn the_console_cookie_is_ignored_when_the_origin_is_the_app() {
    let console = SessionToken::generate();
    let app = SessionToken::generate();
    for origin in ["https://a.edu", "https://app.b.edu", "null"] {
        let h = with_cookies(
            Some(origin),
            &[(COOKIE_NAME, &app), (CONSOLE_COOKIE_NAME, &console)],
        );
        assert!(
            matches!(
                credential(&h, &server(&["https://app.b.edu"])),
                Ok((Channel::SessionCookie, t)) if t == app
            ),
            "{origin}"
        );
    }
    // With no app cookie to fall back on, a console cookie from another origin is no credential.
    let alone = with_cookies(Some("https://a.edu"), &[(CONSOLE_COOKIE_NAME, &console)]);
    assert!(matches!(
        credential(&alone, &server(&[])),
        Err(AuthError::Unauthenticated)
    ));
}

#[test]
fn a_malformed_console_cookie_is_rejected_not_skipped() {
    let app = SessionToken::generate();
    let mut h = HeaderMap::new();
    h.insert(
        header::COOKIE,
        HeaderValue::from_str(&format!(
            "{CONSOLE_COOKIE_NAME}=junk; {COOKIE_NAME}={}",
            app.expose()
        ))
        .unwrap(),
    );
    assert!(matches!(
        credential(&h, &server(&[])),
        Err(AuthError::Unauthenticated)
    ));
}

#[test]
fn both_cookies_are_tried_console_first_so_a_dead_one_does_not_shadow_the_other() {
    let console = SessionToken::generate();
    let app = SessionToken::generate();
    let h = with_cookies(
        None,
        &[(COOKIE_NAME, &app), (CONSOLE_COOKIE_NAME, &console)],
    );
    let found = candidates(&h, &server(&[])).unwrap();
    let channels: Vec<Channel> = found.iter().map(|(channel, _)| *channel).collect();
    assert_eq!(channels, [Channel::ConsoleCookie, Channel::SessionCookie]);
    let from_app = with_cookies(
        Some("https://a.edu"),
        &[(COOKIE_NAME, &app), (CONSOLE_COOKIE_NAME, &console)],
    );
    let found = candidates(&from_app, &server(&[])).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, Channel::SessionCookie);
}

#[test]
fn a_channel_speaks_for_one_audience() {
    assert_eq!(Channel::Bearer.audience(), Audience::App);
    assert_eq!(Channel::SessionCookie.audience(), Audience::App);
    assert_eq!(Channel::ConsoleCookie.audience(), Audience::Console);
}
