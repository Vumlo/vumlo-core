use axum::http::HeaderValue;

use super::*;

fn server(allowed: &[&str]) -> ServerSettings {
    ServerSettings {
        bind_addr: ([127, 0, 0, 1], 0).into(),
        public_origin: "https://a.edu".to_owned(),
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

#[test]
fn read_only_methods_need_no_origin() {
    for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
        assert!(check_origin(&method, &headers(None), &server(&[])).is_ok());
    }
}

#[test]
fn unsafe_methods_need_the_exact_origin() {
    for method in [Method::POST, Method::PATCH, Method::DELETE, Method::PUT] {
        assert!(check_origin(&method, &headers(Some("https://a.edu")), &server(&[])).is_ok());
        for bad in [
            None,
            Some("https://evil.test"),
            Some("https://a.edu.evil.test"),
            Some("null"),
            Some("https://a.edu/"),
        ] {
            assert!(
                matches!(
                    check_origin(&method, &headers(bad), &server(&["https://app.b.edu"])),
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
    for method in [Method::POST, Method::PATCH, Method::DELETE, Method::PUT] {
        for good in [
            "https://a.edu",
            "https://app.b.edu",
            "http://localhost:5173",
        ] {
            assert!(check_origin(&method, &headers(Some(good)), &server).is_ok());
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
                    check_origin(&method, &headers(Some(bad)), &server),
                    Err(AuthError::OriginMismatch)
                ),
                "{method} {bad}"
            );
        }
    }
}

#[test]
fn bearer_wins_over_cookie_and_junk_is_rejected() {
    let token = SessionToken::generate();
    let other = SessionToken::generate();
    let mut h = HeaderMap::new();
    h.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("bearer {}", token.expose())).unwrap(),
    );
    h.insert(
        header::COOKIE,
        HeaderValue::from_str(&format!("{COOKIE_NAME}={}", other.expose())).unwrap(),
    );
    assert!(matches!(credential(&h), Ok(Credential::Bearer(t)) if t == token));

    for bad in ["Basic abc", "Bearer", "Bearer short", "token"] {
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, HeaderValue::from_static(bad));
        assert!(
            matches!(credential(&h), Err(AuthError::Unauthenticated)),
            "{bad}"
        );
    }
}

#[test]
fn cookie_is_read_and_missing_credentials_are_rejected() {
    let token = SessionToken::generate();
    let mut h = HeaderMap::new();
    h.insert(
        header::COOKIE,
        HeaderValue::from_str(&format!("theme=dark; {COOKIE_NAME}={}", token.expose())).unwrap(),
    );
    assert!(matches!(credential(&h), Ok(Credential::Cookie(t)) if t == token));
    assert!(matches!(
        credential(&HeaderMap::new()),
        Err(AuthError::Unauthenticated)
    ));
}
