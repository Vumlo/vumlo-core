use super::*;

const KEY: [u8; 32] = [7; 32];

fn sample() -> OptOut {
    OptOut {
        user_id: Uuid::now_v7(),
        plugin_id: "deadline-change-alert".to_owned(),
        kind: "deadline_changed".to_owned(),
    }
}

#[test]
fn a_signed_token_names_the_same_person_plugin_and_kind() {
    let opt_out = sample();
    let token = sign(&KEY, &opt_out).unwrap();
    assert!(
        token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    );
    assert_eq!(verify(&KEY, &token), Some(opt_out));
}

#[test]
fn a_token_signed_with_another_key_or_changed_is_refused() {
    let token = sign(&KEY, &sample()).unwrap();
    assert_eq!(verify(&[8; 32], &token), None);
    // Another payload under the old tag.
    let other = sign(&KEY, &sample()).unwrap();
    let forged = format!(
        "{}.{}",
        other.split_once('.').unwrap().0,
        token.split_once('.').unwrap().1
    );
    assert_eq!(verify(&KEY, &forged), None);
}

#[test]
fn garbage_is_refused_without_a_panic() {
    for bad in ["", ".", "a.b", "no-dot", "%%%.%%%", "....", "djF8eA.AAAA"] {
        assert_eq!(verify(&KEY, bad), None, "{bad}");
    }
}

#[test]
fn a_token_with_a_wrong_shape_under_a_valid_signature_is_refused() {
    for payload in [
        "v2|11111111-1111-1111-1111-111111111111|p|k",
        "v1|not-a-uuid|p|k",
        "v1|11111111-1111-1111-1111-111111111111|p",
        "v1|11111111-1111-1111-1111-111111111111|p|k|extra",
        "v1|11111111-1111-1111-1111-111111111111||k",
    ] {
        let tag = mac(&KEY, payload.as_bytes())
            .unwrap()
            .finalize()
            .into_bytes();
        let token = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(payload),
            URL_SAFE_NO_PAD.encode(tag)
        );
        assert_eq!(verify(&KEY, &token), None, "{payload}");
    }
}
