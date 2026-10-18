use super::*;

fn settings(dir: &std::path::Path, from: &str) -> MailSettings {
    MailSettings {
        from: from.to_owned(),
        link_base: None,
        transport: MailTransport::File {
            dir: dir.to_owned(),
        },
    }
}

#[tokio::test]
async fn a_message_lands_in_the_directory_with_its_recipient_subject_and_body() {
    let dir = std::env::temp_dir().join(format!("vumlo-mail-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let mailer = Mailer::new(
        &settings(&dir, "Vumlo <no-reply@lms.test>"),
        "https://lms.test",
    )
    .unwrap();

    mailer
        .send(
            "Maya Okafor",
            "maya@lms.test",
            "Hello",
            "Body line".to_owned(),
        )
        .await
        .unwrap();

    let files: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
    assert_eq!(files.len(), 1);
    let text = std::fs::read_to_string(files[0].as_ref().unwrap().path()).unwrap();
    for needle in [
        "maya@lms.test",
        "no-reply@lms.test",
        "Subject: Hello",
        "Body line",
    ] {
        assert!(text.contains(needle), "{needle}: {text}");
    }
    assert_eq!(
        mailer.link("/verify-email?token=t"),
        "https://lms.test/verify-email?token=t"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_bad_sender_is_refused_with_the_key_and_the_value() {
    let error = Mailer::new(
        &settings(std::path::Path::new("."), "not a mailbox"),
        "https://x.test",
    )
    .err()
    .unwrap()
    .to_string();
    assert!(
        error.contains("mail.from") && error.contains("not a mailbox"),
        "{error}"
    );
}
