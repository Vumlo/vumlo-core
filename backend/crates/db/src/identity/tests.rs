use super::*;

fn policy() -> PasswordPolicy {
    PasswordPolicy::new(
        12,
        "anna.kowalska@school.edu",
        "Anna",
        "Kowalska",
        "Example University",
    )
}

fn rejection(raw: &str) -> Vec<String> {
    match Password::parse(raw.to_owned(), &policy()) {
        Err(ModelError::Password(reasons)) => reasons,
        other => panic!("expected a rejection, got {other:?}"),
    }
}

#[test]
fn email_is_trimmed_and_lowercased() {
    assert_eq!(
        Email::parse("  Anna.K+lms@School.EDU ").unwrap().as_str(),
        "anna.k+lms@school.edu"
    );
}

#[test]
fn email_rejects_bad_syntax() {
    for bad in [
        "",
        "plain",
        "@school.edu",
        "a@",
        "a@school",
        "a b@school.edu",
        "a@@school.edu",
        "a@school..edu",
        "a@-school.edu",
        ".a@school.edu",
        "a..b@school.edu",
        "<a>@school.edu",
        "a@school.edu.",
        "a@sch ool.edu",
    ] {
        assert_eq!(Email::parse(bad), Err(ModelError::Email), "{bad:?}");
    }
    let long_local = format!("{}@school.edu", "a".repeat(65));
    assert!(Email::parse(&long_local).is_err());
    let long_total = format!("a@{}.edu", "b".repeat(250));
    assert!(Email::parse(&long_total).is_err());
}

#[test]
fn names_are_trimmed_and_bounded_and_each_reports_its_own_field() {
    assert_eq!(PersonName::name("  Anna  ").unwrap().as_str(), "Anna");
    assert_eq!(
        PersonName::surname(" Kowalska ").unwrap().as_str(),
        "Kowalska"
    );
    for bad in ["   ", "", "a\nb", &"ł".repeat(101)] {
        assert_eq!(PersonName::name(bad), Err(ModelError::Name), "{bad:?}");
        assert_eq!(
            PersonName::surname(bad),
            Err(ModelError::Surname),
            "{bad:?}"
        );
    }
    assert!(PersonName::name(&"ł".repeat(100)).is_ok());
}

#[test]
fn eleven_chars_rejected_and_the_limit_is_configurable() {
    let reasons = rejection("Tr0ub4dor&3");
    assert_eq!(reasons, ["must be at least 12 characters"]);
    let relaxed = PasswordPolicy {
        min_len: 8,
        ..policy()
    };
    assert!(Password::parse("vermilion-quartz-lantern".to_owned(), &relaxed).is_ok());
}

#[test]
fn common_password_rejected_with_feedback() {
    let reasons = rejection("password1234");
    assert!(!reasons.is_empty());
    assert!(reasons.iter().all(|r| !r.contains("password1234")));
}

#[test]
fn password_built_from_user_inputs_is_rejected() {
    rejection("annakowalska");
    let without_inputs = PasswordPolicy {
        user_inputs: Vec::new(),
        ..policy()
    };
    assert!(
        Password::parse("annakowalska".to_owned(), &without_inputs).is_ok(),
        "the same password is only weak because it matches the user's own data"
    );
}

#[test]
fn long_passphrase_with_spaces_and_unicode_is_accepted() {
    let ok = Password::parse(
        "zażółć gęślą jaźń — wieczorem nad łąką".to_owned(),
        &policy(),
    );
    assert!(ok.is_ok(), "{ok:?}");
    assert!(Password::parse("correct horse battery staple".to_owned(), &policy()).is_ok());
}

#[test]
fn password_longer_than_128_chars_is_rejected() {
    let long = "correct horse battery staple ".repeat(5);
    assert!(long.chars().count() > PASSWORD_MAX_LEN);
    assert_eq!(rejection(&long), ["must be at most 128 characters"]);
}

#[test]
fn secrets_are_never_printed() {
    let password = Password::parse("correct horse battery staple".to_owned(), &policy()).unwrap();
    let temp = TemporaryPassword::generate();
    let hashed = hash("correct horse battery staple").unwrap();
    for printed in [
        format!("{password:?}"),
        format!("{temp:?}"),
        format!("{hashed:?}"),
    ] {
        assert!(printed.contains("***"), "{printed}");
    }
    assert!(!format!("{temp:?}").contains(temp.expose()));
    assert!(!format!("{hashed:?}").contains(hashed.as_str()));
}

#[test]
fn temporary_password_has_128_bits_of_entropy_as_22_chars() {
    let temp = TemporaryPassword::generate();
    assert_eq!(temp.expose().len(), 22);
    assert_ne!(temp, TemporaryPassword::generate());
}

#[test]
fn a_hash_is_an_argon2id_phc_string_with_the_owasp_parameters() {
    let hashed = hash("correct horse battery staple").unwrap();
    assert!(
        hashed
            .as_str()
            .starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
        "{}",
        hashed.as_str()
    );
}
