use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};

use super::*;

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.append(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    map
}

#[test]
fn only_a_post_to_the_api_outside_auth_is_covered() {
    assert!(applies(
        &Method::POST,
        "/api/v1/assignments/x/submission/submit"
    ));
    assert!(!applies(
        &Method::PUT,
        "/api/v1/assignments/x/submission/draft"
    ));
    assert!(!applies(&Method::POST, "/api/v1/auth/login"));
    assert!(!applies(&Method::POST, "/api/v1/auth/google/callback"));
    assert!(!applies(&Method::POST, "/elsewhere"));
}

#[test]
fn a_key_is_one_to_255_printable_characters_given_once() {
    assert_eq!(key_of(&headers(&[])).unwrap(), None);
    assert_eq!(
        key_of(&headers(&[("idempotency-key", "0190f1c2-7b1e")]))
            .unwrap()
            .as_deref(),
        Some("0190f1c2-7b1e")
    );
    for bad in ["", "has space", &"k".repeat(256)] {
        assert!(
            key_of(&headers(&[("idempotency-key", bad)])).is_err(),
            "{bad:?}"
        );
    }
    let twice = headers(&[("idempotency-key", "a"), ("idempotency-key", "b")]);
    assert!(key_of(&twice).is_err());
    assert!(
        key_of(&headers(&[("idempotency-key", &"k".repeat(255))]))
            .unwrap()
            .is_some()
    );
}

#[test]
fn the_scope_follows_the_credential_and_its_kind() {
    let bearer = scope_of(&headers(&[("authorization", "Bearer abc")]));
    let other = scope_of(&headers(&[("authorization", "Bearer abd")]));
    let cookie = scope_of(&headers(&[("cookie", "vumlo_session=abc")]));
    assert!(bearer.is_some() && other.is_some() && cookie.is_some());
    assert_ne!(bearer, other);
    assert_ne!(bearer, cookie);
    assert_eq!(scope_of(&headers(&[("cookie", "other=abc")])), None);
    assert_eq!(scope_of(&headers(&[])), None);
}

#[test]
fn only_an_answer_about_the_request_is_kept() {
    for kept in [200, 201, 204, 404, 409, 422] {
        assert!(storable(StatusCode::from_u16(kept).unwrap()), "{kept}");
    }
    for dropped in [301, 401, 408, 429, 500, 503] {
        assert!(
            !storable(StatusCode::from_u16(dropped).unwrap()),
            "{dropped}"
        );
    }
}

#[test]
fn an_answer_that_says_no_store_is_not_kept() {
    for credential in [
        "no-store",
        "No-Store",
        "private, no-store",
        "no-store , max-age=0",
    ] {
        assert!(
            carries_credential(&headers(&[("cache-control", credential)])),
            "{credential}"
        );
    }
    let two_headers = headers(&[("cache-control", "private"), ("cache-control", "no-store")]);
    assert!(carries_credential(&two_headers));
    for plain in ["private, no-cache", "max-age=0", "no-stored"] {
        assert!(
            !carries_credential(&headers(&[("cache-control", plain)])),
            "{plain}"
        );
    }
    assert!(!carries_credential(&headers(&[])));
}
