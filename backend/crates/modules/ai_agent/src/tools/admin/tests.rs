use serde_json::json;

use super::*;

fn parse(name: &str, args: Value) -> Result<Write, String> {
    Write::parse(name, &args.to_string())
}

#[test]
fn each_change_is_read_strictly_and_a_stray_field_is_refused() {
    let (g, u) = (Uuid::now_v7(), Uuid::now_v7());
    let term = parse(
        "create_term",
        json!({"name": "T", "starts_on": "2026-09-01", "ends_on": "2027-01-31"}),
    )
    .unwrap();
    assert_eq!(term.name(), "create_term");
    assert!(!term.wants_password());
    assert!(parse("create_group", json!({"name": "7B"})).is_ok());
    assert!(parse("add_group_members", json!({"group_id": g, "user_ids": [u]})).is_ok());
    let reset = parse("reset_password", json!({"user_id": u})).unwrap();
    assert!(reset.wants_password(), "only a reset needs a password");

    for (name, bad) in [
        (
            "create_term",
            json!({"name": "T", "starts_on": "soon", "ends_on": "2027-01-31"}),
        ),
        (
            "create_term",
            json!({"name": "T", "starts_on": "2026-09-01"}),
        ),
        ("create_group", json!({"name": "7B", "role": "admin"})),
        (
            "reset_password",
            json!({"user_id": u, "password": "hunter2"}),
        ),
        ("reset_password", json!({"user_id": "not-an-id"})),
        ("add_group_members", json!({"group_id": g})),
    ] {
        assert!(parse(name, bad.clone()).is_err(), "{name} {bad}");
    }
}

#[test]
fn a_group_gets_between_one_and_fifty_people_at_a_time() {
    let g = Uuid::now_v7();
    let ids = |n: usize| (0..n).map(|_| Uuid::now_v7()).collect::<Vec<_>>();
    assert!(
        parse(
            "add_group_members",
            json!({"group_id": g, "user_ids": ids(0)})
        )
        .is_err()
    );
    assert!(
        parse(
            "add_group_members",
            json!({"group_id": g, "user_ids": ids(50)})
        )
        .is_ok()
    );
    assert!(
        parse(
            "add_group_members",
            json!({"group_id": g, "user_ids": ids(51)})
        )
        .is_err()
    );
}

#[test]
fn a_stored_change_reads_back_the_same_and_stores_no_password() {
    let u = Uuid::now_v7();
    let reset = parse("reset_password", json!({"user_id": u})).unwrap();
    let stored = serde_json::to_string(&reset).unwrap();
    assert_eq!(
        stored,
        format!(r#"{{"tool":"reset_password","user_id":"{u}"}}"#)
    );
    let again: Write = serde_json::from_str(&stored).unwrap();
    assert!(again.wants_password());
}
