//! What the layer writes to the log (CLAUDE.md section 11: never an address, only ids and classes).

use std::io::Write;
use std::sync::{Arc, Mutex};

use test_support::db_test;
use uuid::Uuid;
use vumlo_config::{Language, MailSettings, MailTransport};

use crate::{
    AccountMail, Address, Body, Context, Kind, Limits, Mailer, PluginMail, Recipient, Subject,
    enqueue_account, enqueue_plugin, process_once,
};

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn failures_and_dead_letters_are_logged_by_id_and_class_never_by_address_or_name() {
    let captured = Captured::default();
    let sink = captured.clone();
    // A guard for this thread only: a #[tokio::test] runs everything on it.
    let _guard = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || sink.clone())
            .finish(),
    );
    db_test(async |pool| {
        // The transport cannot write, so the first round is a retry.
        let dir = std::env::temp_dir().join(format!("vumlo-mail-log-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("file");
        std::fs::write(&blocker, "x").unwrap();
        let settings = MailSettings::new("no-reply@lms.test", MailTransport::File { dir: blocker });
        let mailer = Mailer::new(&settings, "https://lms.test").unwrap();
        let context = Context::new(&settings, Language::En, "Test University");

        let mut conn = pool.acquire().await.unwrap();
        let user = Uuid::now_v7();
        enqueue_plugin(
            &mut conn,
            &Limits {
                plugin_hourly_calls: 10,
                recipient_daily: 10,
            },
            PluginMail {
                plugin_id: "p".to_owned(),
                kind: Kind::parse("k").unwrap(),
                subject: Subject::parse("Subject of Maya Okafor").unwrap(),
                body: Body::parse("Body for Maya Okafor").unwrap(),
                link_path: None,
                dedupe_key: None,
                recipients: vec![Recipient {
                    user_id: user,
                    name: "Maya Okafor".to_owned(),
                    address: Address::parse("maya.okafor@lms.test").unwrap(),
                }],
                single: true,
                no_address: 0,
                disabled: 0,
            },
        )
        .await
        .unwrap();
        enqueue_account(
            &mut conn,
            AccountMail {
                origin_id: Uuid::now_v7(),
                kind: Kind::parse("verify-email").unwrap(),
                user_id: Uuid::now_v7(),
                name: "Omar Haddad".to_owned(),
                address: Address::parse("omar.haddad@lms.test").unwrap(),
                subject: Subject::parse("Confirm").unwrap(),
                body: Body::parse("Open the link.").unwrap(),
            },
        )
        .await
        .unwrap();
        // One retries, one is given up on.
        sqlx::query!(
            "UPDATE mail.messages SET expires_at = now() - interval '1 minute'
             WHERE plugin_id IS NULL"
        )
        .execute(&mut *conn)
        .await
        .unwrap();
        drop(conn);
        assert_eq!(process_once(&pool, &mailer, &context).await.unwrap(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(
            captured_text(&captured).contains(&user.to_string()),
            "the user id is what a log line names"
        );
    })
    .await;
    let log = captured_text(&captured);
    assert!(log.contains("mail failed, will retry"), "{log}");
    assert!(log.contains("mail given up"), "{log}");
    for leaked in [
        "maya", "Maya", "okafor", "omar", "Omar", "haddad", "lms.test",
    ] {
        assert!(!log.contains(leaked), "`{leaked}` is in the log:\n{log}");
    }
}

fn captured_text(captured: &Captured) -> String {
    String::from_utf8(captured.0.lock().unwrap().clone()).unwrap()
}
