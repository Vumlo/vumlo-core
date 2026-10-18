use super::*;

#[test]
fn every_mail_has_the_link_the_name_and_the_institution_in_both_languages() {
    for purpose in [TokenPurpose::Verify, TokenPurpose::Reset] {
        for language in [Language::En, Language::Uk] {
            let (subject, body) = text(
                purpose,
                language,
                "Maya",
                "Northfield",
                "https://x.test/a?token=t",
            );
            assert!(!subject.is_empty());
            for needle in ["Maya", "Northfield", "https://x.test/a?token=t"] {
                assert!(body.contains(needle), "{purpose:?} {language:?} {needle}");
            }
        }
    }
}
