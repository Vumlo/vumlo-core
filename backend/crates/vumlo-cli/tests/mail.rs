//! `vumlo-cli mail ...` (ADR-0087): the journal is readable without an address anywhere, and `retry`
//! puts only the dead messages that can still be sent back, with one audit row.

mod common;

use common::{Env, audit, audit_meta, db_test};
use mail::{Address, Body, Kind, Limits, PluginMail, Recipient, Subject};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

const LIMITS: Limits = Limits {
    plugin_hourly_calls: 100,
    recipient_daily: 100,
};

struct Seeded {
    /// Dead, address kept.
    retryable: Uuid,
    /// Dead, address already erased.
    erased: Uuid,
    pending: Uuid,
    person: Uuid,
}

fn person(label: &str) -> Recipient {
    Recipient {
        user_id: Uuid::now_v7(),
        name: label.to_owned(),
        address: Address::parse(&format!("{label}@lms.test")).unwrap(),
    }
}

async fn message_of(pool: &PgPool, user: Uuid) -> Uuid {
    sqlx::query_scalar!(
        r#"SELECT id AS "id!" FROM mail.messages WHERE user_id = $1"#,
        user
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn seed(pool: &PgPool) -> Seeded {
    let people = [person("ada"), person("bob"), person("cy")];
    let ids = people.each_ref().map(|p| p.user_id);
    let mut conn = pool.acquire().await.unwrap();
    mail::enqueue_plugin(
        &mut conn,
        &LIMITS,
        PluginMail {
            plugin_id: "deadline-alert".to_owned(),
            kind: Kind::parse("deadline_changed").unwrap(),
            subject: Subject::parse("Deadline moved").unwrap(),
            body: Body::parse("Now Friday.").unwrap(),
            link_path: None,
            dedupe_key: None,
            recipients: people.into(),
            single: false,
            no_address: 0,
            disabled: 0,
        },
    )
    .await
    .unwrap();
    drop(conn);
    let seeded = Seeded {
        retryable: message_of(pool, ids[0]).await,
        erased: message_of(pool, ids[1]).await,
        pending: message_of(pool, ids[2]).await,
        person: ids[0],
    };
    sqlx::query!(
        "UPDATE mail.messages SET state = 'dead', finished_at = now(), attempts = 5,
                last_error = 'smtp 451' WHERE id = ANY($1)",
        &[seeded.retryable, seeded.erased]
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query!(
        "UPDATE mail.messages SET to_address = NULL, to_name = NULL WHERE id = $1",
        seeded.erased
    )
    .execute(pool)
    .await
    .unwrap();
    seeded
}

async fn state_of(pool: &PgPool, id: Uuid) -> String {
    sqlx::query_scalar!(
        r#"SELECT state AS "s!" FROM mail.messages WHERE id = $1"#,
        id
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn status_and_list_show_the_journal_and_never_an_address() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let seeded = seed(&pool).await;
        let env = Env::new(&pool);

        let status = env.run(&["mail", "status"]).await;
        assert!(status.ok, "{}", status.stderr);
        assert!(
            status.stdout.contains("pending        1"),
            "{}",
            status.stdout
        );
        assert!(
            status.stdout.contains("dead, in all   2"),
            "{}",
            status.stdout
        );
        assert!(
            status.stdout.contains("deadline-alert"),
            "{}",
            status.stdout
        );
        let json = env.run(&["mail", "status", "--json"]).await;
        let value: Value = serde_json::from_str(&json.stdout).unwrap();
        assert_eq!(
            (value["pending"].as_i64(), value["dead"].as_i64()),
            (Some(1), Some(2))
        );

        assert!(status.stdout.contains("not configured"), "{}", status.stdout);
        assert_eq!(value["transport"], Value::Null);
        let with_mail = Env::with_config(
            &pool,
            &format!("{}\n[mail]\nfrom = \"no-reply@lms.test\"\ntransport = {{ kind = \"file\", dir = \"mail\" }}\n", common::CONFIG),
        );
        let configured = with_mail.run(&["mail", "status"]).await;
        assert!(configured.ok, "{}", configured.stderr);
        assert!(configured.stdout.contains("transport      file"), "{}", configured.stdout);

        let list = env.run(&["mail", "list"]).await;
        assert!(list.ok, "{}", list.stderr);
        for needle in [
            seeded.retryable.to_string(),
            seeded.pending.to_string(),
            seeded.person.to_string(),
            "Deadline moved".to_owned(),
            "smtp 451".to_owned(),
        ] {
            assert!(list.stdout.contains(&needle), "{needle}: {}", list.stdout);
        }
        let dead = env
            .run(&[
                "mail",
                "list",
                "--state",
                "dead",
                "--plugin",
                "deadline-alert",
                "--json",
            ])
            .await;
        let items: Vec<Value> = serde_json::from_str(&dead.stdout).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| item["state"] == "dead"));
        let one = env
            .run(&[
                "mail",
                "list",
                "--user",
                &seeded.person.to_string(),
                "--json",
            ])
            .await;
        assert_eq!(
            serde_json::from_str::<Vec<Value>>(&one.stdout)
                .unwrap()
                .len(),
            1
        );
        let none = env.run(&["mail", "list", "--kind", "other"]).await;
        assert!(
            none.stdout.contains("no message matches"),
            "{}",
            none.stdout
        );
        assert!(!env.run(&["mail", "list", "--state", "bogus"]).await.ok);

        for out in [&status, &json, &list, &dead, &one] {
            assert!(!out.stdout.contains("@lms.test"), "{}", out.stdout);
            assert!(!out.stderr.contains("@lms.test"), "{}", out.stderr);
        }
        assert!(audit(&pool).await.is_empty(), "reading writes no audit row");
    })
    .await;
}

#[tokio::test]
async fn retry_puts_back_what_can_be_sent_and_audits_it_once() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let seeded = seed(&pool).await;
        let env = Env::new(&pool);

        let dry = env.run(&["mail", "retry", "--dead", "--dry-run"]).await;
        assert!(
            dry.ok && dry.stdout.contains("would put back 1"),
            "{}",
            dry.stdout
        );
        assert!(
            dry.stdout.contains(&seeded.erased.to_string()),
            "{}",
            dry.stdout
        );
        assert_eq!(state_of(&pool, seeded.retryable).await, "dead");
        assert!(audit(&pool).await.is_empty());

        let run = env
            .run(&["mail", "retry", "--dead", "--plugin", "deadline-alert"])
            .await;
        assert!(run.ok, "{}", run.stderr);
        assert!(run.stdout.contains("put back 1"), "{}", run.stdout);
        assert_eq!(state_of(&pool, seeded.retryable).await, "pending");
        assert_eq!(state_of(&pool, seeded.erased).await, "dead");
        assert_eq!(audit(&pool).await.len(), 1);
        let meta = audit_meta(&pool, "mail.retry").await;
        assert_eq!(meta["count"], 1);
        assert_eq!(meta["ids"][0], seeded.retryable.to_string());
        assert!(!meta.to_string().contains("@lms.test"));

        // Nothing left that can be sent: no change and no second row.
        let again = env.run(&["mail", "retry", "--dead"]).await;
        assert!(
            again.ok && again.stdout.contains("put back 0"),
            "{}",
            again.stdout
        );
        let by_id = env
            .run(&["mail", "retry", &seeded.pending.to_string()])
            .await;
        assert!(
            by_id.stdout.contains("nothing was changed"),
            "{}",
            by_id.stdout
        );
        assert_eq!(state_of(&pool, seeded.pending).await, "pending");
        assert_eq!(audit(&pool).await.len(), 1);

        // The command needs an id or --dead, and --plugin only goes with --dead.
        assert!(!env.run(&["mail", "retry"]).await.ok);
        assert!(!env.run(&["mail", "retry", "--plugin", "x"]).await.ok);
    })
    .await;
}
