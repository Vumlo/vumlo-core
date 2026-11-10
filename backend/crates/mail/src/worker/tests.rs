use std::path::{Path, PathBuf};

use sqlx::PgPool;
use test_support::db_test;
use uuid::Uuid;
use vumlo_config::{Language, MailTransport};

use super::*;
use crate::model::{AccountMail, Address, Body, DedupeKey, Kind, Limits, LinkPath, PluginMail};
use crate::model::{Recipient, Subject};
use crate::service;

const LIMITS: Limits = Limits {
    plugin_hourly_calls: 1000,
    recipient_daily: 1000,
};

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vumlo-mail-worker-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn settings(dir: &Path) -> MailSettings {
    MailSettings::new(
        "Vumlo <no-reply@lms.test>",
        MailTransport::File {
            dir: dir.to_owned(),
        },
    )
}

struct Rig {
    dir: PathBuf,
    mailer: Mailer,
    context: Context,
}

impl Rig {
    fn new() -> Self {
        let dir = temp_dir();
        let settings = settings(&dir);
        Self {
            mailer: Mailer::new(&settings, "https://lms.test").unwrap(),
            context: Context::new(&settings, Language::En, "Test University"),
            dir,
        }
    }

    /// A rig whose transport cannot write: its "directory" is a file.
    fn broken() -> Self {
        let dir = temp_dir();
        let blocker = dir.join("not-a-directory");
        std::fs::write(&blocker, "x").unwrap();
        let settings = settings(&blocker);
        Self {
            mailer: Mailer::new(&settings, "https://lms.test").unwrap(),
            context: Context::new(&settings, Language::En, "Test University"),
            dir,
        }
    }

    fn sent(&self) -> Vec<String> {
        std::fs::read_dir(&self.dir)
            .unwrap()
            .filter_map(|entry| std::fs::read_to_string(entry.unwrap().path()).ok())
            .filter(|text| text.contains("Subject:"))
            .collect()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn person(label: &str) -> Recipient {
    Recipient {
        user_id: Uuid::now_v7(),
        name: label.to_owned(),
        address: Address::parse(&format!("{label}@lms.test")).unwrap(),
    }
}

async fn queue_plugin_mail(pool: &PgPool, who: Recipient) {
    let mut conn = pool.acquire().await.unwrap();
    service::enqueue_plugin(
        &mut conn,
        &LIMITS,
        PluginMail {
            plugin_id: "deadline-alert".to_owned(),
            kind: Kind::parse("deadline_changed").unwrap(),
            subject: Subject::parse("Deadline moved").unwrap(),
            body: Body::parse("The deadline is now Friday.").unwrap(),
            link_path: Some(LinkPath::parse("/courses/42").unwrap()),
            dedupe_key: Some(DedupeKey::parse("a1").unwrap()),
            recipients: vec![who],
            single: false,
            no_address: 0,
            disabled: 0,
        },
    )
    .await
    .unwrap();
}

async fn queue_account_mail(pool: &PgPool) {
    let mut conn = pool.acquire().await.unwrap();
    service::enqueue_account(
        &mut conn,
        AccountMail {
            origin_id: Uuid::now_v7(),
            kind: Kind::parse("verify-email").unwrap(),
            user_id: Uuid::now_v7(),
            name: "Maya".to_owned(),
            address: Address::parse("maya@lms.test").unwrap(),
            subject: Subject::parse("Confirm your address").unwrap(),
            body: Body::parse("Open https://lms.test/verify?token=abc").unwrap(),
        },
    )
    .await
    .unwrap();
}

struct Row {
    state: String,
    attempts: i32,
    last_error: Option<String>,
    has_address: bool,
    erase_in_future: bool,
    retry_in_future: bool,
}

async fn only_row(pool: &PgPool) -> Row {
    let row = sqlx::query!(
        r#"SELECT state, attempts, last_error, to_address IS NOT NULL AS "has_address!",
                  COALESCE(erase_at > now() + interval '1 day', false) AS "erase_in_future!",
                  run_after > now() AS "retry_in_future!"
           FROM mail.messages"#
    )
    .fetch_one(pool)
    .await
    .unwrap();
    Row {
        state: row.state,
        attempts: row.attempts,
        last_error: row.last_error,
        has_address: row.has_address,
        erase_in_future: row.erase_in_future,
        retry_in_future: row.retry_in_future,
    }
}

#[tokio::test]
async fn a_plugin_mail_is_sent_with_its_link_footer_and_headers_and_keeps_its_address_for_the_retention()
 {
    db_test(async |pool| {
        let rig = Rig::new();
        queue_plugin_mail(&pool, person("maya")).await;
        assert_eq!(
            process_once(&pool, &rig.mailer, &rig.context)
                .await
                .unwrap(),
            1
        );
        let sent = rig.sent();
        assert_eq!(sent.len(), 1);
        let text = &sent[0];
        for needle in [
            "maya@lms.test",
            "Subject: Deadline moved",
            "The deadline is now Friday.",
            "https://lms.test/courses/42",
            "deadline-alert",
            "Test University",
            "https://lms.test/api/v1/mail/opt-out/",
            "List-Unsubscribe: <https://lms.test/api/v1/mail/opt-out/",
            "List-Unsubscribe-Post: List-Unsubscribe=One-Click",
            "X-Vumlo-Plugin: deadline-alert",
            "Auto-Submitted: auto-generated",
        ] {
            assert!(text.contains(needle), "{needle}: {text}");
        }
        let row = only_row(&pool).await;
        assert_eq!(row.state, "sent");
        assert_eq!(row.attempts, 1);
        assert!(
            row.has_address && row.erase_in_future,
            "kept for retention_days"
        );
        // Nothing left to do.
        assert_eq!(
            process_once(&pool, &rig.mailer, &rig.context)
                .await
                .unwrap(),
            0
        );
        assert_eq!(rig.sent().len(), 1);
    })
    .await;
}

#[tokio::test]
async fn the_opt_out_link_in_the_mail_works_for_that_person_only() {
    db_test(async |pool| {
        let rig = Rig::new();
        let maya = person("maya");
        queue_plugin_mail(&pool, maya.clone()).await;
        process_once(&pool, &rig.mailer, &rig.context)
            .await
            .unwrap();
        let text = rig.sent().remove(0);
        let token = text
            .split("/api/v1/mail/opt-out/")
            .nth(1)
            .unwrap()
            .split(|c: char| c.is_whitespace() || c == '>')
            .next()
            .unwrap()
            .to_owned();
        // Mail clients may fold the line; the token has no `=` line breaks in a short link.
        assert!(!token.contains('='), "{token}");
        let mut conn = pool.acquire().await.unwrap();
        let named = service::apply_token(&mut conn, &token, true)
            .await
            .unwrap()
            .expect("our own link");
        assert_eq!(named.user_id, maya.user_id);
        assert_eq!(named.plugin_id, "deadline-alert");
        assert_eq!(named.kind, "deadline_changed");
    })
    .await;
}

#[tokio::test]
async fn account_mail_is_sent_as_composed_without_a_footer_and_its_address_goes_at_once() {
    db_test(async |pool| {
        let rig = Rig::new();
        queue_account_mail(&pool).await;
        process_once(&pool, &rig.mailer, &rig.context)
            .await
            .unwrap();
        let text = rig.sent().remove(0);
        assert!(text.contains("https://lms.test/verify?token=abc"), "{text}");
        for absent in ["List-Unsubscribe", "opt-out", "X-Vumlo-Plugin"] {
            assert!(!text.contains(absent), "{absent}: {text}");
        }
        assert_eq!(only_row(&pool).await.state, "sent");
        sweep(&pool, &rig.context).await.unwrap();
        let row = only_row(&pool).await;
        assert!(
            !row.has_address,
            "its text holds a one-time link: no reason to keep it"
        );
        // The text goes with the last address.
        let body = sqlx::query_scalar!("SELECT body FROM mail.batches")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(body, None);
    })
    .await;
}

#[tokio::test]
async fn a_person_who_opted_out_after_queueing_gets_nothing_and_the_message_is_cancelled() {
    db_test(async |pool| {
        let rig = Rig::new();
        let maya = person("maya");
        queue_plugin_mail(&pool, maya.clone()).await;
        let mut conn = pool.acquire().await.unwrap();
        repo::set_opt_out(
            &mut conn,
            maya.user_id,
            "deadline-alert",
            "deadline_changed",
        )
        .await
        .unwrap();
        drop(conn);
        process_once(&pool, &rig.mailer, &rig.context)
            .await
            .unwrap();
        assert!(rig.sent().is_empty());
        assert_eq!(only_row(&pool).await.state, "cancelled");
    })
    .await;
}

#[tokio::test]
async fn a_transient_failure_is_retried_later_with_a_class_and_no_address() {
    db_test(async |pool| {
        let rig = Rig::broken();
        queue_plugin_mail(&pool, person("maya")).await;
        process_once(&pool, &rig.mailer, &rig.context)
            .await
            .unwrap();
        let row = only_row(&pool).await;
        assert_eq!(row.state, "pending");
        assert_eq!(row.attempts, 1);
        assert_eq!(row.last_error.as_deref(), Some("file"));
        assert!(row.retry_in_future, "backed off, not hammered");
        assert!(row.has_address, "still needed for the next attempt");
        // Not due: the next round claims nothing.
        assert_eq!(
            process_once(&pool, &rig.mailer, &rig.context)
                .await
                .unwrap(),
            0
        );

        // The backoff grows with the attempts and the transport recovers.
        sqlx::query!("UPDATE mail.messages SET run_after = now()")
            .execute(&pool)
            .await
            .unwrap();
        let good = Rig::new();
        assert_eq!(
            process_once(&pool, &good.mailer, &good.context)
                .await
                .unwrap(),
            1
        );
        let row = only_row(&pool).await;
        assert_eq!((row.state.as_str(), row.attempts), ("sent", 2));
        assert_eq!(good.sent().len(), 1);
    })
    .await;
}

#[tokio::test]
async fn a_permanent_failure_is_a_dead_letter_at_once_and_names_only_the_class() {
    db_test(async |pool| {
        let rig = Rig::new();
        queue_plugin_mail(&pool, person("maya")).await;
        // An address the transport refuses (it passed `Address` when it was queued, so go round it).
        sqlx::query!("UPDATE mail.messages SET to_address = 'not an address'")
            .execute(&pool)
            .await
            .unwrap();
        process_once(&pool, &rig.mailer, &rig.context)
            .await
            .unwrap();
        let row = only_row(&pool).await;
        assert_eq!(row.state, "dead");
        assert_eq!(row.last_error.as_deref(), Some("address"));
        assert!(rig.sent().is_empty());
        assert_eq!(
            process_once(&pool, &rig.mailer, &rig.context)
                .await
                .unwrap(),
            0
        );
    })
    .await;
}

#[tokio::test]
async fn a_message_older_than_its_lifetime_is_a_dead_letter_not_a_late_mail() {
    db_test(async |pool| {
        let rig = Rig::new();
        queue_account_mail(&pool).await;
        sqlx::query!("UPDATE mail.messages SET expires_at = now() - interval '1 minute'")
            .execute(&pool)
            .await
            .unwrap();
        process_once(&pool, &rig.mailer, &rig.context)
            .await
            .unwrap();
        let row = only_row(&pool).await;
        assert_eq!(
            (row.state.as_str(), row.last_error.as_deref()),
            ("dead", Some("expired"))
        );
        assert!(rig.sent().is_empty());
    })
    .await;
}

#[tokio::test]
async fn a_claimed_message_is_leased_so_a_second_worker_does_not_send_it_twice() {
    db_test(async |pool| {
        queue_account_mail(&pool).await;
        let mut conn = pool.acquire().await.unwrap();
        let first = repo::claim(&mut conn, 20, 300.0).await.unwrap();
        let second = repo::claim(&mut conn, 20, 300.0).await.unwrap();
        assert_eq!((first.len(), second.len()), (1, 0));
        assert_eq!(first[0].attempts, 1);
        // A crashed worker's message comes back once the lease has run out.
        sqlx::query!("UPDATE mail.messages SET run_after = now() - interval '1 second'")
            .execute(&mut *conn)
            .await
            .unwrap();
        let third = repo::claim(&mut conn, 20, 300.0).await.unwrap();
        assert_eq!((third.len(), third[0].attempts), (1, 2));
    })
    .await;
}

#[tokio::test]
async fn a_round_takes_at_most_twenty_messages() {
    db_test(async |pool| {
        let rig = Rig::new();
        for _ in 0..25 {
            queue_account_mail(&pool).await;
        }
        assert_eq!(
            process_once(&pool, &rig.mailer, &rig.context)
                .await
                .unwrap(),
            20
        );
        assert_eq!(
            process_once(&pool, &rig.mailer, &rig.context)
                .await
                .unwrap(),
            5
        );
        assert_eq!(rig.sent().len(), 25);
    })
    .await;
}

#[tokio::test]
async fn the_sweep_erases_due_addresses_then_the_text_then_old_rows_and_dedupe_claims() {
    db_test(async |pool| {
        let rig = Rig::new();
        queue_plugin_mail(&pool, person("maya")).await;
        process_once(&pool, &rig.mailer, &rig.context)
            .await
            .unwrap();

        // Within the retention nothing is erased.
        sweep(&pool, &rig.context).await.unwrap();
        assert!(only_row(&pool).await.has_address);

        sqlx::query!("UPDATE mail.messages SET erase_at = now() - interval '1 second'")
            .execute(&pool)
            .await
            .unwrap();
        sweep(&pool, &rig.context).await.unwrap();
        let after = sqlx::query!(
            "SELECT m.to_address, m.to_name, b.subject, b.body, b.link_path
             FROM mail.messages m JOIN mail.batches b ON b.id = m.batch_id"
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((after.to_address, after.to_name), (None, None));
        assert_eq!(
            (after.subject, after.body, after.link_path),
            (None, None, None)
        );
        assert_eq!(only_row(&pool).await.state, "sent", "the journal row stays");

        // The journal is pruned after journal_days, and the dedupe claim after 30 days.
        sqlx::query!(
            "UPDATE mail.batches SET created_at = now() - make_interval(days => $1)",
            rig.context.journal_days + 1
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!("UPDATE mail.dedupe SET at = now() - interval '31 days'")
            .execute(&pool)
            .await
            .unwrap();
        sweep(&pool, &rig.context).await.unwrap();
        let left = sqlx::query!(
            r#"SELECT (SELECT count(*) FROM mail.messages) AS "messages!",
                      (SELECT count(*) FROM mail.batches) AS "batches!",
                      (SELECT count(*) FROM mail.dedupe) AS "dedupe!""#
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((left.messages, left.batches, left.dedupe), (0, 0, 0));
    })
    .await;
}

#[tokio::test]
async fn a_batch_keeps_its_text_while_any_of_its_addresses_remains() {
    db_test(async |pool| {
        let rig = Rig::new();
        let mut conn = pool.acquire().await.unwrap();
        service::enqueue_plugin(
            &mut conn,
            &LIMITS,
            PluginMail {
                plugin_id: "p".to_owned(),
                kind: Kind::parse("k").unwrap(),
                subject: Subject::parse("s").unwrap(),
                body: Body::parse("b").unwrap(),
                link_path: None,
                dedupe_key: None,
                recipients: vec![person("a"), person("b")],
                single: false,
                no_address: 0,
                disabled: 0,
            },
        )
        .await
        .unwrap();
        drop(conn);
        process_once(&pool, &rig.mailer, &rig.context)
            .await
            .unwrap();
        sqlx::query!(
            "UPDATE mail.messages SET erase_at = now() - interval '1 second'
             WHERE id = (SELECT id FROM mail.messages LIMIT 1)"
        )
        .execute(&pool)
        .await
        .unwrap();
        sweep(&pool, &rig.context).await.unwrap();
        let body = sqlx::query_scalar!("SELECT body FROM mail.batches")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(body.as_deref(), Some("b"));
    })
    .await;
}
