use super::*;

#[test]
fn title_is_trimmed_and_bounded() {
    assert_eq!(
        CourseTitle::parse("  Algebra 7-A ").unwrap().as_str(),
        "Algebra 7-A"
    );
    assert!(CourseTitle::parse("").is_err());
    assert!(CourseTitle::parse("   ").is_err());
    assert!(CourseTitle::parse("a\nb").is_err());
    assert!(CourseTitle::parse(&"ł".repeat(200)).is_ok());
    assert!(CourseTitle::parse(&"ł".repeat(201)).is_err());
}

#[test]
fn member_source_round_trips() {
    for source in [MemberSource::Direct, MemberSource::Group] {
        assert_eq!(source.as_str().parse::<MemberSource>(), Ok(source));
    }
    assert!("linked".parse::<MemberSource>().is_err());
}

#[test]
fn course_kind_round_trips() {
    for kind in [CourseKind::Regular, CourseKind::Template] {
        assert_eq!(kind.as_str().parse::<CourseKind>(), Ok(kind));
    }
    assert!("course".parse::<CourseKind>().is_err());
}
