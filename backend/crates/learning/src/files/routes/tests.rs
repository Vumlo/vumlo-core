use super::*;

#[test]
fn a_range_is_first_last_inclusive_and_clamped_to_the_file() {
    assert_eq!(parse_range("bytes=0-9", 100), Some(Ok((0, 9))));
    assert_eq!(parse_range("bytes=90-", 100), Some(Ok((90, 99))));
    assert_eq!(parse_range("bytes=90-500", 100), Some(Ok((90, 99))));
    assert_eq!(parse_range("bytes=-10", 100), Some(Ok((90, 99))));
    assert_eq!(parse_range("bytes=-500", 100), Some(Ok((0, 99))));
}

#[test]
fn a_range_outside_the_file_is_refused_and_an_unusable_one_is_ignored() {
    assert_eq!(parse_range("bytes=100-", 100), Some(Err(())));
    assert_eq!(parse_range("bytes=-0", 100), Some(Err(())));
    for ignored in [
        "bytes=5-2",
        "bytes=0-1,4-5",
        "bytes=abc-",
        "items=0-1",
        "bytes=",
        "garbage",
    ] {
        assert_eq!(parse_range(ignored, 100), None, "{ignored}");
    }
}

#[test]
fn a_name_cannot_break_out_of_the_disposition() {
    let value = disposition("a\"; filename=evil.exe\r\nX: y%.pdf", true);
    let text = value.to_str().expect("an ASCII header value");
    assert_eq!(
        text,
        "inline; filename=\"a__ filename=evil.exe__X: y_.pdf\"; \
         filename*=UTF-8''a%22%3B%20filename%3Devil.exe%0D%0AX%3A%20y%25.pdf"
    );
    let value = disposition("Звіт з фізики.pdf", false);
    assert!(
        value
            .to_str()
            .expect("ASCII")
            .starts_with("attachment; filename=\"____ _ ______.pdf\"; filename*=UTF-8''%D0%97")
    );
}

#[test]
fn if_none_match_matches_a_list_a_weak_tag_and_a_star() {
    let with = |value: &str| {
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, value.parse().expect("header value"));
        headers
    };
    assert!(matches_etag(&with("\"a\", \"b\""), "\"b\""));
    assert!(matches_etag(&with("W/\"b\""), "\"b\""));
    assert!(matches_etag(&with("*"), "\"b\""));
    assert!(!matches_etag(&with("\"c\""), "\"b\""));
    assert!(!matches_etag(&HeaderMap::new(), "\"b\""));
}
