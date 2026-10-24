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
