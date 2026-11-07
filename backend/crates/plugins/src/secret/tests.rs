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
