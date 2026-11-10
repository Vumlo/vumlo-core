use super::*;

fn settings(dir: &std::path::Path, from: &str) -> MailSettings {
    MailSettings::new(
        from,
        MailTransport::File {
            dir: dir.to_owned(),
        },
    )
}

fn temp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vumlo-mail-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn only_message(dir: &std::path::Path) -> String {
    let files: Vec<_> = std::fs::read_dir(dir).unwrap().collect();
    assert_eq!(files.len(), 1);
    std::fs::read_to_string(files[0].as_ref().unwrap().path()).unwrap()
}

#[tokio::test]
async fn a_message_lands_in_the_directory_with_its_recipient_subject_and_body() {
    let dir = temp_dir();
    let mailer = Mailer::new(
        &settings(&dir, "Vumlo <no-reply@lms.test>"),
        "https://lms.test",
    )
    .unwrap();
    mailer
        .send(Outgoing {
            name: "Maya Okafor",
            address: "maya@lms.test",
            subject: "Hello",
            body: "Body line".to_owned(),
            plugin: None,
        })
        .await
        .unwrap();
    let text = only_message(&dir);
    for needle in [
        "maya@lms.test",
        "no-reply@lms.test",
        "Subject: Hello",
        "Body line",
    ] {
        assert!(text.contains(needle), "{needle}: {text}");
    }
    // Account mail has none of the plugin headers.
    for header in ["List-Unsubscribe", "X-Vumlo-Plugin", "Auto-Submitted"] {
        assert!(!text.contains(header), "{header}: {text}");
    }
    assert_eq!(
        mailer.link("/verify-email?token=t"),
        "https://lms.test/verify-email?token=t"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_plugin_message_carries_the_one_click_and_origin_headers() {
    let dir = temp_dir();
    let mailer = Mailer::new(&settings(&dir, "no-reply@lms.test"), "https://lms.test").unwrap();
    mailer
        .send(Outgoing {
            name: "Maya",
            address: "maya@lms.test",
            subject: "Hi",
            body: "x".to_owned(),
            plugin: Some(("text-metrics", "https://lms.test/api/v1/mail/opt-out/T")),
        })
        .await
        .unwrap();
    let text = only_message(&dir);
    for needle in [
        "List-Unsubscribe: <https://lms.test/api/v1/mail/opt-out/T>",
        "List-Unsubscribe-Post: List-Unsubscribe=One-Click",
        "X-Vumlo-Plugin: text-metrics",
        "Auto-Submitted: auto-generated",
    ] {
        assert!(text.contains(needle), "{needle}: {text}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn an_address_the_transport_refuses_is_a_permanent_error_with_a_class_only() {
    let dir = temp_dir();
    let mailer = Mailer::new(&settings(&dir, "no-reply@lms.test"), "https://lms.test").unwrap();
    let error = mailer
        .send(Outgoing {
            name: "X",
            address: "not an address",
            subject: "s",
            body: "b".to_owned(),
            plugin: None,
        })
        .await
        .unwrap_err();
    assert!(error.is_permanent());
    assert_eq!(error.class(), "address");
    assert!(!error.to_string().contains("not an address"));
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
