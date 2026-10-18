use super::*;

fn max(text: &str) -> Decimal {
    text.parse().unwrap()
}

#[test]
fn a_score_is_zero_up_to_max_with_two_decimals() {
    for ok in ["0", "0.00", "0.01", "7.5", "19.99", "20", "20.00"] {
        assert!(Score::parse(ok, max("20")).is_ok(), "{ok}");
    }
    for bad in [
        "-0.01", "-1", "20.01", "21", "1.005", "1e1", "", " 5", "5 ", "abc", "+5",
    ] {
        assert_eq!(
            Score::parse(bad, max("20")),
            Err(ModelError::Score { max: max("20") }),
            "{bad:?}"
        );
    }
}

#[test]
fn the_score_message_names_the_max_with_two_decimals() {
    let error = Score::parse("21", max("20")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "must be a decimal string from 0 to 20.00 with at most two decimals"
    );
    let half = Score::parse("13", max("12.5")).unwrap_err();
    assert!(half.to_string().contains("12.50"), "{half}");
}

#[test]
fn feedback_is_trimmed_bounded_plain_text_and_empty_means_none() {
    assert_eq!(
        Feedback::parse("  good  ").unwrap().unwrap().as_str(),
        "good"
    );
    assert_eq!(
        Feedback::parse("line\n\tnext").unwrap().unwrap().as_str(),
        "line\n\tnext"
    );
    for empty in ["", "   ", "\n\t "] {
        assert_eq!(Feedback::parse(empty), Ok(None), "{empty:?}");
    }
    assert!(Feedback::parse(&"ł".repeat(4000)).unwrap().is_some());
    assert_eq!(
        Feedback::parse(&"ł".repeat(4001)),
        Err(ModelError::Feedback)
    );
    for bad in ["a\u{0}b", "a\u{7f}b", "a\u{1b}[0m"] {
        assert_eq!(Feedback::parse(bad), Err(ModelError::Feedback), "{bad:?}");
    }
}

#[test]
fn the_column_status_round_trips() {
    for status in [ColumnStatus::Draft, ColumnStatus::Published] {
        assert_eq!(status.as_str().parse::<ColumnStatus>(), Ok(status));
    }
}
