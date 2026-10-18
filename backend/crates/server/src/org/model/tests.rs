use super::*;

#[test]
fn a_name_is_trimmed_and_bounded() {
    assert_eq!(OrgName::parse("  7-A ").unwrap().as_str(), "7-A");
    assert!(OrgName::parse("").is_err());
    assert!(OrgName::parse("   ").is_err());
    assert!(OrgName::parse("a\tb").is_err());
    assert!(OrgName::parse(&"ł".repeat(100)).is_ok());
    assert!(OrgName::parse(&"ł".repeat(101)).is_err());
}

#[test]
fn a_term_may_last_one_day_but_not_end_before_it_starts() {
    let day = |d| NaiveDate::from_ymd_opt(2026, 9, d).unwrap();
    assert!(TermDates::new(day(1), day(1)).is_ok());
    assert!(TermDates::new(day(1), day(30)).is_ok());
    assert_eq!(TermDates::new(day(2), day(1)), Err(ModelError::Dates));
}

#[test]
fn a_term_lies_in_the_years_1_to_9999() {
    let date = |year| NaiveDate::from_ymd_opt(year, 6, 1).unwrap();
    assert!(TermDates::new(date(1), date(9999)).is_ok());
    assert_eq!(
        TermDates::new(date(0), date(2026)),
        Err(ModelError::StartsOnRange)
    );
    assert_eq!(
        TermDates::new(date(-5000), date(2026)),
        Err(ModelError::StartsOnRange)
    );
    assert_eq!(
        TermDates::new(date(2026), date(10_000)),
        Err(ModelError::EndsOnRange)
    );
}
