use super::*;

#[test]
fn a_tag_follows_the_body_and_is_weak() {
    assert_eq!(etag(b"{}"), etag(b"{}"));
    assert_ne!(etag(b"{}"), etag(b"{ }"));
    let tag = etag(b"{}");
    assert!(tag.starts_with("W/\"") && tag.ends_with('"'));
    assert_eq!(tag.len(), 3 + 32 + 1);
}

#[test]
fn if_none_match_compares_weakly_and_takes_a_list() {
    let tag = etag(b"x");
    let strong = tag.strip_prefix("W/").unwrap().to_owned();
    assert!(matches(std::slice::from_ref(&tag), &tag));
    assert!(
        matches(&[strong], &tag),
        "W/\"a\" and \"a\" are the same tag"
    );
    assert!(matches(&["W/\"other\"".to_owned(), tag.clone()], &tag));
    assert!(matches(&["*".to_owned()], &tag));
    assert!(!matches(&["W/\"other\"".to_owned()], &tag));
    assert!(!matches(&[], &tag));
}
