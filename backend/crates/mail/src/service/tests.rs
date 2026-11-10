use sqlx::PgPool;
use test_support::db_test;

use super::*;
use crate::model::{
    AccountMail, Address, Body, DedupeKey, Kind, Limits, MessageState, PluginMail, Recipient,
    Subject,
};

const LIMITS: Limits = Limits {
    plugin_hourly_calls: 3,
    recipient_daily: 2,
};

fn person(label: &str) -> Recipient {
    Recipient {
        user_id: Uuid::now_v7(),
        name: label.to_owned(),
        address: Address::parse(&format!("{label}@lms.test")).unwrap(),
    }
}

fn mail(plugin: &str, kind: &str, recipients: Vec<Recipient>) -> PluginMail {
    PluginMail {
        plugin_id: plugin.to_owned(),
        kind: Kind::parse(kind).unwrap(),
        subject: Subject::parse("Deadline moved").unwrap(),
        body: Body::parse("The deadline is now Friday.").unwrap(),
        link_path: None,
        dedupe_key: None,
        recipients,
        single: false,
        no_address: 0,
        disabled: 0,
    }
}

fn account(origin_id: Uuid, user_id: Uuid) -> AccountMail {
    AccountMail {
        origin_id,
        kind: Kind::parse("verify-email").unwrap(),
        user_id,
        name: "Maya".to_owned(),
        address: Address::parse("maya@lms.test").unwrap(),
        subject: Subject::parse("Confirm your address").unwrap(),
        body: Body::parse("Open the link.").unwrap(),
    }
}

async fn count(pool: &PgPool, state: &str) -> i64 {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM mail.messages WHERE state = $1"#,
        state
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn an_account_mail_is_queued_once_per_origin_event() {
    db_test(async |pool| {
        let mut conn = pool.acquire().await.unwrap();
        let (origin, user) = (Uuid::now_v7(), Uuid::now_v7());
        assert!(!origin_queued(&mut conn, origin).await.unwrap());
        assert!(
            enqueue_account(&mut conn, account(origin, user))
                .await
                .unwrap()
        );
        assert!(origin_queued(&mut conn, origin).await.unwrap());
        // The same outbox event again, as a rerun of its handler would.
        assert!(
            !enqueue_account(&mut conn, account(origin, user))
                .await
                .unwrap()
        );
        assert_eq!(count(&pool, "pending").await, 1);
        let batches = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM mail.batches"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(batches, 1, "the losing call leaves no empty batch");
        let row = sqlx::query!("SELECT plugin_id, to_address FROM mail.messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.plugin_id, None);
        assert_eq!(row.to_address.as_deref(), Some("maya@lms.test"));
    })
    .await;
}

#[tokio::test]
async fn a_plugin_mail_queues_a_row_per_recipient_and_counts_the_skipped_reasons() {
    db_test(async |pool| {
        let mut conn = pool.acquire().await.unwrap();
        let (a, b) = (person("a"), person("b"));
        let mut request = mail(
            "deadline-alert",
            "deadline_changed",
            vec![a.clone(), b.clone(), a],
        );
        request.no_address = 2;
        request.disabled = 1;
        let outcome = enqueue_plugin(&mut conn, &LIMITS, request).await.unwrap();
        // A person listed twice is mailed once.
        assert_eq!(outcome.accepted, 2);
        assert_eq!(outcome.skipped.no_address, 2);
        assert_eq!(outcome.skipped.disabled, 1);
        assert_eq!(count(&pool, "pending").await, 2);
        let batch = sqlx::query!("SELECT accepted, skipped, plugin_id FROM mail.batches")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(batch.accepted, 2);
        assert_eq!(batch.plugin_id.as_deref(), Some("deadline-alert"));
        assert_eq!(batch.skipped["no_address"], 2);
        assert_eq!(
            batch.skipped["rate_limited"], 0,
            "all five reasons are stored"
        );
    })
    .await;
}

#[tokio::test]
async fn a_person_who_opted_out_gets_nothing_of_that_plugin_and_kind_only() {
    db_test(async |pool| {
        let mut conn = pool.acquire().await.unwrap();
        let (maya, omar) = (person("maya"), person("omar"));
        repo::set_opt_out(
            &mut conn,
            maya.user_id,
            "deadline-alert",
            "deadline_changed",
        )
        .await
        .unwrap();
        let outcome = enqueue_plugin(
            &mut conn,
            &LIMITS,
            mail(
                "deadline-alert",
                "deadline_changed",
                vec![maya.clone(), omar.clone()],
            ),
        )
        .await
        .unwrap();
        assert_eq!((outcome.accepted, outcome.skipped.opted_out), (1, 1));
        // Another kind of the same plugin, and the same kind of another plugin, still arrive.
        for (plugin, kind) in [
            ("deadline-alert", "reminder"),
            ("other-plugin", "deadline_changed"),
        ] {
            let outcome =
                enqueue_plugin(&mut conn, &LIMITS, mail(plugin, kind, vec![maya.clone()]))
                    .await
                    .unwrap();
            assert_eq!(outcome.accepted, 1, "{plugin} {kind}");
        }
    })
    .await;
}

#[tokio::test]
async fn the_daily_limit_is_an_error_for_one_person_and_a_skip_in_an_audience() {
    db_test(async |pool| {
        let mut conn = pool.acquire().await.unwrap();
        let (maya, omar) = (person("maya"), person("omar"));
        for _ in 0..2 {
            enqueue_plugin(
                &mut conn,
                &Limits {
                    plugin_hourly_calls: 100,
                    ..LIMITS
                },
                mail("p", "k", vec![maya.clone()]),
            )
            .await
            .unwrap();
        }
        let limits = Limits {
            plugin_hourly_calls: 100,
            ..LIMITS
        };
        let mut single = mail("p", "k", vec![maya.clone()]);
        single.single = true;
        match enqueue_plugin(&mut conn, &limits, single)
            .await
            .unwrap_err()
        {
            MailError::RecipientLimit { retry_after_secs } => {
                assert!(
                    (1..=86_400).contains(&retry_after_secs),
                    "{retry_after_secs}"
                );
            }
            other => panic!("{other:?}"),
        }
        // In an audience the same person is skipped and the rest go.
        let outcome = enqueue_plugin(
            &mut conn,
            &limits,
            mail("p", "k", vec![maya.clone(), omar.clone()]),
        )
        .await
        .unwrap();
        assert_eq!((outcome.accepted, outcome.skipped.rate_limited), (1, 1));
        // Account mail does not count towards the daily limit, and plugin mail does not stop it.
        enqueue_account(&mut conn, account(Uuid::now_v7(), maya.user_id))
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
async fn a_message_that_ended_still_counts_towards_the_daily_limit() {
    db_test(async |pool| {
        let mut conn = pool.acquire().await.unwrap();
        let limits = Limits {
            plugin_hourly_calls: 100,
            recipient_daily: 1,
        };
        let maya = person("maya");
        enqueue_plugin(&mut conn, &limits, mail("p", "k", vec![maya.clone()]))
            .await
            .unwrap();
        sqlx::query!("UPDATE mail.messages SET state = 'sent', finished_at = now()")
            .execute(&mut *conn)
            .await
            .unwrap();
        let outcome = enqueue_plugin(&mut conn, &limits, mail("p", "k", vec![maya]))
            .await
            .unwrap();
        assert_eq!((outcome.accepted, outcome.skipped.rate_limited), (0, 1));
    })
    .await;
}

#[tokio::test]
async fn the_hourly_limit_counts_accepted_calls_and_says_when_to_retry() {
    db_test(async |pool| {
        let mut conn = pool.acquire().await.unwrap();
        let limits = Limits {
            plugin_hourly_calls: 3,
            recipient_daily: 100,
        };
        for _ in 0..3 {
            enqueue_plugin(&mut conn, &limits, mail("busy", "k", vec![person("x")]))
                .await
                .unwrap();
        }
        let error = enqueue_plugin(&mut conn, &limits, mail("busy", "k", vec![person("y")]))
            .await
            .unwrap_err();
        let MailError::HourlyLimit { retry_after_secs } = error else {
            panic!("{error:?}");
        };
        assert!(
            (3500..=3600).contains(&retry_after_secs),
            "{retry_after_secs}"
        );
        assert_eq!(
            count(&pool, "pending").await,
            3,
            "a refused call queues nothing"
        );
        // Another plugin has its own allowance.
        enqueue_plugin(&mut conn, &limits, mail("quiet", "k", vec![person("z")]))
            .await
            .unwrap();
        // Once the oldest call is more than an hour old the plugin may send again.
        sqlx::query!(
            "UPDATE mail.batches SET created_at = now() - interval '61 minutes'
             WHERE plugin_id = 'busy'"
        )
        .execute(&mut *conn)
        .await
        .unwrap();
        enqueue_plugin(&mut conn, &limits, mail("busy", "k", vec![person("y")]))
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
async fn a_dedupe_key_is_accepted_once_per_plugin_and_person_for_thirty_days() {
    db_test(async |pool| {
        let mut conn = pool.acquire().await.unwrap();
        let limits = Limits {
            plugin_hourly_calls: 100,
            recipient_daily: 100,
        };
        let (maya, omar) = (person("maya"), person("omar"));
        let keyed = |plugin: &str, who: Vec<Recipient>| {
            let mut m = mail(plugin, "k", who);
            m.dedupe_key = Some(DedupeKey::parse("assignment-7").unwrap());
            m
        };
        let first = enqueue_plugin(&mut conn, &limits, keyed("p", vec![maya.clone()]))
            .await
            .unwrap();
        assert_eq!(first.accepted, 1);
        let again = enqueue_plugin(
            &mut conn,
            &limits,
            keyed("p", vec![maya.clone(), omar.clone()]),
        )
        .await
        .unwrap();
        assert_eq!(
            (again.accepted, again.skipped.duplicate),
            (1, 1),
            "only Omar is new"
        );
        // The key belongs to the plugin.
        let other = enqueue_plugin(&mut conn, &limits, keyed("q", vec![maya.clone()]))
            .await
            .unwrap();
        assert_eq!(other.accepted, 1);
        // After thirty days the key is free again.
        sqlx::query!("UPDATE mail.dedupe SET at = now() - interval '31 days'")
            .execute(&mut *conn)
            .await
            .unwrap();
        let later = enqueue_plugin(&mut conn, &limits, keyed("p", vec![maya]))
            .await
            .unwrap();
        assert_eq!(later.accepted, 1);
    })
    .await;
}

#[tokio::test]
async fn a_skipped_message_claims_no_dedupe_key() {
    db_test(async |pool| {
        let mut conn = pool.acquire().await.unwrap();
        let limits = Limits {
            plugin_hourly_calls: 100,
            recipient_daily: 100,
        };
        let maya = person("maya");
        repo::set_opt_out(&mut conn, maya.user_id, "p", "k")
            .await
            .unwrap();
        let mut request = mail("p", "k", vec![maya.clone()]);
        request.dedupe_key = Some(DedupeKey::parse("once").unwrap());
        let outcome = enqueue_plugin(&mut conn, &limits, request).await.unwrap();
        assert_eq!(outcome.skipped.opted_out, 1);
        let claimed = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM mail.dedupe"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(claimed, 0);
        // Back in, the same key is still accepted.
        repo::clear_opt_out(&mut conn, maya.user_id, "p", "k")
            .await
            .unwrap();
        let mut request = mail("p", "k", vec![maya]);
        request.dedupe_key = Some(DedupeKey::parse("once").unwrap());
        assert_eq!(
            enqueue_plugin(&mut conn, &limits, request)
                .await
                .unwrap()
                .accepted,
            1
        );
    })
    .await;
}

#[tokio::test]
async fn two_concurrent_calls_with_one_dedupe_key_queue_one_message() {
    db_test(async |pool| {
        let limits = Limits {
            plugin_hourly_calls: 100,
            recipient_daily: 100,
        };
        let maya = person("maya");
        let call = |maya: Recipient| {
            let pool = pool.clone();
            async move {
                let mut tx = pool.begin().await.unwrap();
                let mut request = mail("p", "k", vec![maya]);
                request.dedupe_key = Some(DedupeKey::parse("same").unwrap());
                let outcome = enqueue_plugin(&mut tx, &limits, request).await.unwrap();
                tx.commit().await.unwrap();
                outcome.accepted
            }
        };
        let (one, two) = tokio::join!(call(maya.clone()), call(maya));
        assert_eq!(one + two, 1);
        assert_eq!(count(&pool, "pending").await, 1);
    })
    .await;
}

#[tokio::test]
async fn a_rolled_back_call_leaves_no_trace_of_its_dedupe_claim_or_rows() {
    db_test(async |pool| {
        let limits = Limits {
            plugin_hourly_calls: 100,
            recipient_daily: 100,
        };
        let maya = person("maya");
        let mut request = mail("p", "k", vec![maya.clone()]);
        request.dedupe_key = Some(DedupeKey::parse("k1").unwrap());
        let mut tx = pool.begin().await.unwrap();
        enqueue_plugin(&mut tx, &limits, request.clone())
            .await
            .unwrap();
        tx.rollback().await.unwrap();
        assert_eq!(count(&pool, "pending").await, 0);
        let mut conn = pool.acquire().await.unwrap();
        assert_eq!(
            enqueue_plugin(&mut conn, &limits, request)
                .await
                .unwrap()
                .accepted,
            1
        );
    })
    .await;
}

#[tokio::test]
async fn cancelling_a_plugin_stops_its_pending_mail_and_erases_the_addresses() {
    db_test(async |pool| {
        let mut conn = pool.acquire().await.unwrap();
        let limits = Limits {
            plugin_hourly_calls: 100,
            recipient_daily: 100,
        };
        enqueue_plugin(
            &mut conn,
            &limits,
            mail("gone", "k", vec![person("a"), person("b")]),
        )
        .await
        .unwrap();
        enqueue_plugin(&mut conn, &limits, mail("stays", "k", vec![person("c")]))
            .await
            .unwrap();
        enqueue_account(&mut conn, account(Uuid::now_v7(), Uuid::now_v7()))
            .await
            .unwrap();
        assert_eq!(cancel_pending(&mut conn, "gone").await.unwrap(), 2);
        assert_eq!(count(&pool, "cancelled").await, 2);
        assert_eq!(
            count(&pool, "pending").await,
            2,
            "other plugins and account mail wait"
        );
        let kept = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM mail.messages
               WHERE plugin_id = 'gone' AND to_address IS NOT NULL AND erase_at <= now()"#
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(kept, 2, "due for erasing at once");
        // Nothing left to cancel the second time.
        assert_eq!(cancel_pending(&mut conn, "gone").await.unwrap(), 0);
    })
    .await;
}

#[tokio::test]
async fn the_signing_key_is_made_once_and_a_token_works_only_with_it() {
    db_test(async |pool| {
        let mut conn = pool.acquire().await.unwrap();
        let first = signing_key(&mut conn).await.unwrap();
        assert_eq!(first.len(), 32);
        assert_eq!(signing_key(&mut conn).await.unwrap(), first);

        let named = OptOut {
            user_id: Uuid::now_v7(),
            plugin_id: "p".to_owned(),
            kind: "k".to_owned(),
        };
        let token = token(&first, &named).unwrap();
        let applied = apply_token(&mut conn, &token, true).await.unwrap();
        assert_eq!(applied, Some(named.clone()));
        assert!(
            repo::is_opted_out(&mut conn, named.user_id, "p", "k")
                .await
                .unwrap()
        );
        // The same token undoes it.
        apply_token(&mut conn, &token, false).await.unwrap();
        assert!(
            !repo::is_opted_out(&mut conn, named.user_id, "p", "k")
                .await
                .unwrap()
        );
        // A token that is not ours writes nothing.
        assert_eq!(
            apply_token(&mut conn, "junk.junk", true).await.unwrap(),
            None
        );
        // Deleting the key voids every link.
        sqlx::query!("DELETE FROM mail.signing_key")
            .execute(&mut *conn)
            .await
            .unwrap();
        assert_eq!(apply_token(&mut conn, &token, true).await.unwrap(), None);
    })
    .await;
}

#[tokio::test]
async fn retry_puts_dead_mail_back_but_leaves_one_whose_address_is_gone() {
    db_test(async |pool| {
        let mut conn = pool.acquire().await.unwrap();
        let limits = Limits {
            plugin_hourly_calls: 100,
            recipient_daily: 100,
        };
        enqueue_plugin(
            &mut conn,
            &limits,
            mail("p", "k", vec![person("a"), person("b")]),
        )
        .await
        .unwrap();
        enqueue_plugin(&mut conn, &limits, mail("q", "k", vec![person("c")]))
            .await
            .unwrap();
        sqlx::query!(
            "UPDATE mail.messages SET state = 'dead', finished_at = now(), attempts = 7,
                    last_error = 'smtp 550'"
        )
        .execute(&mut *conn)
        .await
        .unwrap();
        // One of the two of plugin p lost its address.
        sqlx::query!(
            "UPDATE mail.messages SET to_address = NULL, to_name = NULL
             WHERE id = (SELECT id FROM mail.messages WHERE plugin_id = 'p' LIMIT 1)"
        )
        .execute(&mut *conn)
        .await
        .unwrap();

        let dry = retry(
            &mut conn,
            &RetryScope::Dead {
                plugin_id: Some("p".to_owned()),
            },
            true,
        )
        .await
        .unwrap();
        assert_eq!((dry.requeued.len(), dry.erased.len()), (1, 1));
        assert_eq!(count(&pool, "dead").await, 3, "a dry run changes nothing");

        let done = retry(
            &mut conn,
            &RetryScope::Dead {
                plugin_id: Some("p".to_owned()),
            },
            false,
        )
        .await
        .unwrap();
        assert_eq!(done.requeued.len(), 1);
        let row = sqlx::query!(
            "SELECT attempts, last_error, finished_at FROM mail.messages WHERE id = $1",
            done.requeued[0]
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.attempts, 0);
        assert_eq!(row.last_error, None);
        assert_eq!(row.finished_at, None);
        assert_eq!(
            count(&pool, "dead").await,
            2,
            "the other plugin's mail is untouched"
        );

        let one = retry(&mut conn, &RetryScope::One(done.erased[0]), false)
            .await
            .unwrap();
        assert!(one.requeued.is_empty());
        assert_eq!(one.erased, done.erased);
    })
    .await;
}

#[tokio::test]
async fn status_and_list_show_state_and_never_an_address() {
    db_test(async |pool| {
        let mut conn = pool.acquire().await.unwrap();
        let limits = Limits {
            plugin_hourly_calls: 100,
            recipient_daily: 100,
        };
        let a = person("secret-name");
        enqueue_plugin(&mut conn, &limits, mail("p", "k1", vec![a.clone()]))
            .await
            .unwrap();
        enqueue_plugin(&mut conn, &limits, mail("p", "k2", vec![person("b")]))
            .await
            .unwrap();
        sqlx::query!(
            "UPDATE mail.messages SET state = 'dead', finished_at = now(), last_error = 'smtp 550'
             WHERE kind = 'k2'"
        )
        .execute(&mut *conn)
        .await
        .unwrap();
        let status = status(&mut conn).await.unwrap();
        assert_eq!(
            (status.pending, status.dead_now, status.dead_24h),
            (1, 1, 1)
        );
        assert_eq!(
            status.dead_by,
            vec![(Some("p".to_owned()), "k2".to_owned(), 1)]
        );
        assert!(status.oldest_pending.is_some());

        let all = list(&mut conn, &Filter::default(), 10).await.unwrap();
        assert_eq!(all.len(), 2);
        let dead = list(
            &mut conn,
            &Filter {
                state: Some(MessageState::Dead),
                plugin_id: Some("p".to_owned()),
                ..Filter::default()
            },
            10,
        )
        .await
        .unwrap();
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].last_error.as_deref(), Some("smtp 550"));
        let by_user = list(
            &mut conn,
            &Filter {
                user_id: Some(a.user_id),
                ..Filter::default()
            },
            10,
        )
        .await
        .unwrap();
        assert_eq!(by_user.len(), 1);
        // The entry type has no address field; its Debug text has none either.
        let text = format!("{all:?}");
        assert!(
            !text.contains("@lms.test") && !text.contains("secret-name"),
            "{text}"
        );
    })
    .await;
}
