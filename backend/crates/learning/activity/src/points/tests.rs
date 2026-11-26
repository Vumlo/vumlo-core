use super::*;

#[test]
fn accepts_up_to_two_decimals() {
    for ok in ["0", "0.00", "20", "7.5", "20.00", "0.01", "9999.99", "0012"] {
        assert!(parse_decimal(ok).is_some(), "{ok}");
    }
}

#[test]
fn rejects_every_other_shape() {
    for bad in [
        "",
        ".5",
        "5.",
        "+5",
        "-1",
        "1e2",
        " 5",
        "5 ",
        "1,5",
        "1.2.3",
        "1.005",
        "abc",
        "99999999999999999999",
    ] {
        assert!(parse_decimal(bad).is_none(), "{bad:?}");
    }
}

#[test]
fn max_points_accepts_decimal_strings_up_to_two_decimals() {
    for ok in ["20", "7.5", "20.00", "0.01", "9999.99", "9999", "0012"] {
        assert!(MaxPoints::parse(ok).is_ok(), "{ok}");
    }
}

#[test]
fn max_points_rejects_everything_else() {
    for bad in [
        "0",
        "0.00",
        "-1",
        "10000",
        "9999.991",
        "10000.00",
        "1.005",
        "abc",
        "",
        ".5",
        "5.",
        "+5",
        "1e2",
        " 5",
        "5 ",
        "1,5",
        "1.2.3",
        "99999999999999999999",
    ] {
        assert_eq!(MaxPoints::parse(bad), Err(PointsError), "{bad:?}");
    }
}

#[test]
fn max_points_display_has_exactly_two_decimals() {
    for (raw, shown) in [
        ("20", "20.00"),
        ("7.5", "7.50"),
        ("20.00", "20.00"),
        ("0.01", "0.01"),
        ("9999.99", "9999.99"),
    ] {
        assert_eq!(MaxPoints::parse(raw).unwrap().to_string(), shown);
    }
}
