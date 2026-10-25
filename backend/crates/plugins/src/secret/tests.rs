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

#[test]
fn an_unset_key_gives_no_secret_key_and_the_key_is_never_shown() {
    assert!(SecretKey::from_config(&Secret::new("")).is_none());
    assert_eq!(format!("{:?}", key()), "SecretKey(***)");
}
