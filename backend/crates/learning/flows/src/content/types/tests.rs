use super::*;

#[test]
fn ids_are_short_and_plain() {
    for ok in ["q1", "a_b-C", &"x".repeat(64)] {
        assert!(valid_id(ok), "{ok}");
    }
    for bad in ["", "a b", "é", "a.b", &"x".repeat(65)] {
        assert!(!valid_id(bad), "{bad:?}");
    }
}

#[test]
fn numbers_take_a_point_or_a_comma_and_nothing_else() {
    assert_eq!(parse_number("3.14"), Some("3.14".parse().unwrap()));
    assert_eq!(parse_number(" -2,5 "), Some("-2.5".parse().unwrap()));
    assert_eq!(parse_number("7"), Some(Decimal::from(7)));
    for bad in [
        "",
        ".",
        "1.",
        ".5",
        "1e3",
        "1 2",
        "--1",
        "1,2,3",
        "abc",
        &"9".repeat(41),
    ] {
        assert_eq!(parse_number(bad), None, "{bad:?}");
    }
}
