use super::*;

#[test]
fn role_text_round_trips() {
    for role in [Role::Admin, Role::Teacher, Role::Student] {
        assert_eq!(role.as_str().parse::<Role>(), Ok(role));
    }
    assert!("owner".parse::<Role>().is_err());
}
