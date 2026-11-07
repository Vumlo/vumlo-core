use super::*;

const BODY: &[u8] = br#"{"id":"e1","type":"submission.submitted","data":{"submission_id":"s1"}}"#;

fn key() -> SecretKey {
    SecretKey::from_config(&Secret::new("0123456789abcdef0123456789abcdef")).unwrap()
}

#[test]
fn a_signature_is_the_hmac_of_the_timestamp_a_dot_and_the_body() {
    // Computed with Python's `hmac`; the example plugin checks the same vector.
    assert_eq!(
        sign("test-secret", 1_700_000_000, BODY).unwrap(),
        "sha256=93d5d9c8289fe4a0066a2634f57443ea75081452aec5ad86e7310daeaf07e941"
    );
}

#[test]
fn a_signature_depends_on_the_secret_the_time_and_every_byte_of_the_body() {
    let base = sign("test-secret", 1_700_000_000, BODY).unwrap();
    assert_ne!(base, sign("other-secret", 1_700_000_000, BODY).unwrap());
    assert_ne!(base, sign("test-secret", 1_700_000_001, BODY).unwrap());
    assert_ne!(base, sign("test-secret", 1_700_000_000, b"{}").unwrap());
    assert!(base.starts_with("sha256=") && base.len() == "sha256=".len() + 64);
}

#[test]
fn the_secret_of_an_install_is_derived_the_same_way_every_time() {
    let id = Uuid::parse_str("018f0000-0000-7000-8000-000000000001").unwrap();
    assert_eq!(
        derive_secret(&key(), id, 1).unwrap(),
        "EIgx9qA-veRMKTomajt8flmBCwGbWhQqlS-7poYOoqQ"
    );
    let first = derive_secret(&key(), id, 1).unwrap();
    assert_ne!(first, derive_secret(&key(), id, 2).unwrap());
    assert_ne!(first, derive_secret(&key(), Uuid::now_v7(), 1).unwrap());
    let other =
        SecretKey::from_config(&Secret::new("a-different-key-of-at-least-32-chars")).unwrap();
    assert_ne!(first, derive_secret(&other, id, 1).unwrap());
}

const LAUNCH_VECTOR: &str = "eyJhc3NpZ25tZW50IjoiMDE4ZjAwMDAtMDAwMC03MDAwLTgwMDAtMDAwMDAwMDAwMDAyIiwiZXhwIjoxNzAwMDAwMDYwLCJpYXQiOjE3MDAwMDAwMDAsInBsdWdpbiI6ImhlbGxvIiwicm9sZSI6InN0YWZmIiwidXNlciI6IjAxOGYwMDAwLTAwMDAtNzAwMC04MDAwLTAwMDAwMDAwMDAwMyIsInYiOjF9.9dfee57a6b11ea16056625ebbc8e28cb9a32e7fada4f7a630218fff1996bc9c0";

fn claims(role: &str) -> LaunchClaims<'_> {
    LaunchClaims {
        plugin: "hello",
        assignment: Uuid::parse_str("018f0000-0000-7000-8000-000000000002").unwrap(),
        user: Uuid::parse_str("018f0000-0000-7000-8000-000000000003").unwrap(),
        role,
        issued_at: 1_700_000_000,
    }
}

#[test]
fn a_launch_token_is_the_payload_and_its_hmac_under_its_own_prefix() {
    // Computed with Python's `hmac`; the example plugin checks the same vector.
    assert_eq!(
        launch_token("test-secret", &claims("staff")).unwrap(),
        LAUNCH_VECTOR
    );
    let (payload, _) = LAUNCH_VECTOR.split_once('.').unwrap();
    let decoded = URL_SAFE_NO_PAD.decode(payload).unwrap();
    let claims: serde_json::Value = serde_json::from_slice(&decoded).unwrap();
    assert_eq!(
        claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(),
        LAUNCH_TTL_SECONDS
    );
    assert_eq!(
        (claims["v"].as_i64(), claims["role"].as_str()),
        (Some(1), Some("staff"))
    );
}

#[test]
fn a_launch_token_depends_on_the_secret_and_on_every_claim() {
    let base = launch_token("test-secret", &claims("staff")).unwrap();
    assert_ne!(
        base,
        launch_token("other-secret", &claims("staff")).unwrap()
    );
    assert_ne!(
        base,
        launch_token("test-secret", &claims("student")).unwrap()
    );
    let mut later = claims("staff");
    later.issued_at += 1;
    assert_ne!(base, launch_token("test-secret", &later).unwrap());
}

#[test]
fn the_signature_of_an_event_is_not_the_signature_of_a_token() {
    // The same bytes, signed as an event would be: the prefix makes the two differ.
    let (payload, token_signature) = LAUNCH_VECTOR.split_once('.').unwrap();
    let as_event = sign("test-secret", 1_700_000_000, payload.as_bytes()).unwrap();
    assert_ne!(as_event.trim_start_matches("sha256="), token_signature);
}

#[test]
fn an_unset_key_gives_no_secret_key_and_the_key_is_never_shown() {
    assert!(SecretKey::from_config(&Secret::new("")).is_none());
    assert_eq!(format!("{:?}", key()), "SecretKey(***)");
}

#[test]
fn a_sealed_value_opens_to_what_was_sealed_whatever_its_length() {
    let id = Uuid::now_v7();
    for plain in ["", "x", "token-123", &"long secret ".repeat(40)] {
        let sealed = seal(&key(), id, "api_token", plain).unwrap();
        assert_eq!(open(&key(), id, "api_token", &sealed).unwrap(), plain);
        assert!(plain.len() < 8 || !sealed.contains(plain));
    }
}

#[test]
fn sealing_the_same_value_twice_gives_two_different_values() {
    let id = Uuid::now_v7();
    assert_ne!(
        seal(&key(), id, "k", "same").unwrap(),
        seal(&key(), id, "k", "same").unwrap()
    );
}

fn bytes_of(sealed: &str) -> Vec<u8> {
    URL_SAFE_NO_PAD.decode(sealed).unwrap()
}

fn text_of(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

#[test]
fn a_sealed_value_is_a_format_byte_a_24_byte_nonce_and_a_ciphertext_with_its_tag() {
    let sealed = bytes_of(&seal(&key(), Uuid::now_v7(), "k", "token").unwrap());
    assert_eq!(sealed[0], 1, "the format version");
    // version + nonce + the five bytes of the text + the 16-byte Poly1305 tag
    assert_eq!(sealed.len(), 1 + 24 + 5 + 16);
}

#[test]
fn a_sealed_value_opens_only_where_and_with_what_it_was_sealed() {
    let id = Uuid::now_v7();
    let sealed = seal(&key(), id, "api_token", "token-123").unwrap();
    assert_eq!(open(&key(), id, "api_token", &sealed).unwrap(), "token-123");
    // Another setting, another install, another key.
    assert!(open(&key(), id, "other_setting", &sealed).is_err());
    assert!(open(&key(), Uuid::now_v7(), "api_token", &sealed).is_err());
    let other =
        SecretKey::from_config(&Secret::new("a-different-key-of-at-least-32-chars")).unwrap();
    assert!(open(&other, id, "api_token", &sealed).is_err());
}

#[test]
fn a_sealed_value_that_was_changed_cut_or_never_sealed_does_not_open() {
    let id = Uuid::now_v7();
    let sealed = seal(&key(), id, "api_token", "token-123").unwrap();
    let bytes = bytes_of(&sealed);
    // One flipped bit in the nonce, the ciphertext and the tag; one in the format byte.
    for at in [1, 1 + 24, bytes.len() - 1, 0] {
        let mut changed = bytes.clone();
        changed[at] ^= 1;
        assert!(
            open(&key(), id, "api_token", &text_of(&changed)).is_err(),
            "byte {at}"
        );
    }
    // A format this code does not know, even if everything else is right.
    let mut future = bytes.clone();
    future[0] = 2;
    assert!(open(&key(), id, "api_token", &text_of(&future)).is_err());
    // Cut short, empty, not base64, and plain text.
    for bad in [
        text_of(&bytes[..bytes.len() - 1]),
        text_of(&bytes[..10]),
        String::new(),
        "not base64!".to_owned(),
        "token-123".to_owned(),
    ] {
        assert!(open(&key(), id, "api_token", &bad).is_err(), "{bad}");
    }
}

fn ids() -> (Uuid, Uuid, Uuid) {
    (
        Uuid::parse_str("018f0000-0000-7000-8000-000000000003").unwrap(),
        Uuid::parse_str("018f0000-0000-7000-8000-000000000004").unwrap(),
        Uuid::parse_str("018f0000-0000-7000-8000-000000000005").unwrap(),
    )
}

fn call<'a>() -> ProxyClaims<'a> {
    let (user, course, nonce) = ids();
    ProxyClaims {
        method: "POST",
        target: "/hello/api/echo?x=1",
        user,
        role: "teacher",
        course: Some(course),
        course_role: Some("assistant"),
        nonce,
        timestamp: 1_700_000_000,
    }
}

#[test]
fn the_signature_of_a_proxied_call_is_the_one_the_example_plugin_computes() {
    // Computed with Python's `hmac` from the lines of the doc comment of `proxy_signature`; the
    // example plugin checks the same vector.
    assert_eq!(
        proxy_signature("test-secret", &call(), br#"{"a":1}"#).unwrap(),
        "sha256=f0550504dfb2f66b16d94c999990f57b5b6c5c1d5ac69cc62076cca03e7da247"
    );
    let (user, _, nonce) = ids();
    let none = ProxyClaims {
        method: "GET",
        target: "/api/summary/x",
        role: "admin",
        course: None,
        course_role: None,
        user,
        nonce,
        timestamp: 1_700_000_000,
    };
    assert_eq!(
        proxy_signature("test-secret", &none, b"").unwrap(),
        "sha256=ef8d29b012c338515b63f520e73271e65d0728cd654bc54006d906800dd845a6"
    );
}

#[test]
fn a_proxied_call_cannot_be_changed_in_any_part_without_changing_its_signature() {
    let body: &[u8] = br#"{"a":1}"#;
    let base = proxy_signature("test-secret", &call(), body).unwrap();
    let other = Uuid::now_v7();
    let changed: Vec<(&str, String)> = vec![
        ("secret", proxy_signature("other", &call(), body).unwrap()),
        (
            "body",
            proxy_signature("test-secret", &call(), b"{}").unwrap(),
        ),
        (
            "method",
            proxy_signature(
                "test-secret",
                &ProxyClaims {
                    method: "PUT",
                    ..call()
                },
                body,
            )
            .unwrap(),
        ),
        (
            "target",
            proxy_signature(
                "test-secret",
                &ProxyClaims {
                    target: "/hello/api/echo?x=2",
                    ..call()
                },
                body,
            )
            .unwrap(),
        ),
        (
            "user",
            proxy_signature(
                "test-secret",
                &ProxyClaims {
                    user: other,
                    ..call()
                },
                body,
            )
            .unwrap(),
        ),
        (
            "role",
            proxy_signature(
                "test-secret",
                &ProxyClaims {
                    role: "admin",
                    ..call()
                },
                body,
            )
            .unwrap(),
        ),
        (
            "course",
            proxy_signature(
                "test-secret",
                &ProxyClaims {
                    course: Some(other),
                    ..call()
                },
                body,
            )
            .unwrap(),
        ),
        (
            "no course",
            proxy_signature(
                "test-secret",
                &ProxyClaims {
                    course: None,
                    ..call()
                },
                body,
            )
            .unwrap(),
        ),
        (
            "course role",
            proxy_signature(
                "test-secret",
                &ProxyClaims {
                    course_role: Some("teacher"),
                    ..call()
                },
                body,
            )
            .unwrap(),
        ),
        (
            "nonce",
            proxy_signature(
                "test-secret",
                &ProxyClaims {
                    nonce: other,
                    ..call()
                },
                body,
            )
            .unwrap(),
        ),
        (
            "time",
            proxy_signature(
                "test-secret",
                &ProxyClaims {
                    timestamp: 1_700_000_001,
                    ..call()
                },
                body,
            )
            .unwrap(),
        ),
    ];
    for (what, signature) in changed {
        assert_ne!(base, signature, "{what} is not covered by the signature");
    }
}

#[test]
fn a_signed_event_a_launch_token_and_a_proxied_call_are_not_one_another() {
    // The same secret signs all three. A proxied call whose canonical text were an event body, or the
    // other way round, must not verify as the other: the first line of the canonical text differs.
    let body = b"x";
    let proxied = proxy_signature("s", &call(), body).unwrap();
    let canonical_as_event = format!("vumlo-proxy\n{}", call().nonce);
    assert_ne!(
        proxied,
        sign("s", 1_700_000_000, canonical_as_event.as_bytes()).unwrap()
    );
    assert_ne!(proxied, sign("s", 1_700_000_000, body).unwrap());
}
