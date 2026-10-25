use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use events::OutboxEvent;
use serde_json::{Value, json};
use test_support::db_test;
use tokio::net::TcpListener;
use uuid::Uuid;
use vumlo_config::Secret;

use super::*;
use crate::manifest::parse_manifest;
use crate::model::{InstallStatus, Installed};
use crate::service::{fan_out, install, set_status};

type Seen = Arc<Mutex<Vec<(HeaderMap, Vec<u8>)>>>;

#[derive(Clone)]
struct Shared {
    seen: Seen,
    status: Arc<AtomicU16>,
}

/// A plugin that records what it is sent and answers with the status the test sets.
struct FakePlugin {
    base: String,
    shared: Shared,
}

impl FakePlugin {
    async fn start(status: u16) -> Self {
        let shared = Shared {
            seen: Arc::default(),
            status: Arc::new(AtomicU16::new(status)),
        };
        let app = Router::new()
            .route(
                "/events",
                post(
                    async |State(shared): State<Shared>, headers: HeaderMap, body: Bytes| {
                        shared.seen.lock().unwrap().push((headers, body.to_vec()));
                        StatusCode::from_u16(shared.status.load(Ordering::SeqCst)).unwrap()
                    },
                ),
            )
            .with_state(shared.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await });
        Self { base, shared }
    }

    fn answer(&self, status: u16) {
        self.shared.status.store(status, Ordering::SeqCst);
    }

    fn requests(&self) -> usize {
        self.shared.seen.lock().unwrap().len()
    }
}

fn key() -> SecretKey {
    SecretKey::from_config(&Secret::new("0123456789abcdef0123456789abcdef")).unwrap()
}

fn deliverer() -> Deliverer {
    Deliverer::new(key()).unwrap()
}

/// Installs `hello` at `base` and owes it one `submission.submitted` event.
async fn owed(pool: &PgPool, base: &str) -> (Installed, OutboxEvent) {
    let manifest = parse_manifest(
        json!({"id": "hello", "version": "1", "api": "1", "subscribes": ["submission.submitted"]})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    let installed = install(&mut conn, &key(), base, &manifest, &[])
        .await
        .unwrap();
    let event = OutboxEvent {
        id: Uuid::now_v7(),
        kind: "submission.submitted".to_owned(),
        version: 1,
        payload: json!({"submission_id": Uuid::now_v7(), "student_id": Uuid::now_v7()}),
    };
    assert_eq!(fan_out(&mut conn, &event).await.unwrap(), 1);
    (installed, event)
}

struct State_ {
    attempts: i32,
    delivered: bool,
    failed: bool,
    last_error: Option<String>,
    due: bool,
}

async fn state(pool: &PgPool) -> State_ {
    let row = sqlx::query!(
        r#"SELECT attempts, delivered_at IS NOT NULL AS "delivered!",
                  failed_at IS NOT NULL AS "failed!", last_error,
                  run_after <= now() AS "due!"
           FROM plugins.deliveries"#
    )
    .fetch_one(pool)
    .await
    .unwrap();
    State_ {
        attempts: row.attempts,
        delivered: row.delivered,
        failed: row.failed,
        last_error: row.last_error,
        due: row.due,
    }
}

async fn make_due(pool: &PgPool) {
    sqlx::query!("UPDATE plugins.deliveries SET run_after = now()")
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn an_event_is_sent_signed_with_the_time_and_the_secret_of_the_install() {
    db_test(async |pool| {
        let plugin = FakePlugin::start(202).await;
        let (installed, event) = owed(&pool, &plugin.base).await;

        assert_eq!(deliverer().once(&pool).await.unwrap(), 1);

        let (headers, body) = plugin.shared.seen.lock().unwrap()[0].clone();
        assert_eq!(headers["content-type"], "application/json");
        let timestamp: i64 = headers["x-vumlo-timestamp"]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((chrono::Utc::now().timestamp() - timestamp).abs() <= 5);
        assert_eq!(
            headers["x-vumlo-signature"].to_str().unwrap(),
            sign(&installed.secret, timestamp, &body).unwrap()
        );
        let sent: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            sent,
            json!({"id": event.id, "type": "submission.submitted", "data": event.payload})
        );
        let after = state(&pool).await;
        assert!(after.delivered && !after.failed && after.attempts == 1);
        // Delivered once: nothing is due now.
        assert_eq!(deliverer().once(&pool).await.unwrap(), 0);
        assert_eq!(plugin.requests(), 1);
    })
    .await;
}

#[tokio::test]
async fn a_plugin_that_refuses_is_tried_again_later_and_the_error_holds_no_secret() {
    db_test(async |pool| {
        let plugin = FakePlugin::start(500).await;
        let (installed, _) = owed(&pool, &plugin.base).await;

        assert_eq!(deliverer().once(&pool).await.unwrap(), 1);

        let failed = state(&pool).await;
        assert!(!failed.delivered && !failed.failed && !failed.due);
        assert_eq!(failed.attempts, 1);
        let error = failed.last_error.unwrap();
        assert_eq!(error, "the plugin answered HTTP 500");
        assert!(!error.contains(&installed.secret) && !error.contains(&plugin.base));
        assert_eq!(deliverer().once(&pool).await.unwrap(), 0, "the retry waits");

        plugin.answer(202);
        make_due(&pool).await;
        assert_eq!(deliverer().once(&pool).await.unwrap(), 1);
        let delivered = state(&pool).await;
        assert!(delivered.delivered && delivered.attempts == 2 && delivered.last_error.is_none());
        assert_eq!(plugin.requests(), 2);
    })
    .await;
}

#[tokio::test]
async fn the_fifth_failure_is_a_dead_letter_that_is_not_sent_again() {
    db_test(async |pool| {
        let plugin = FakePlugin::start(503).await;
        owed(&pool, &plugin.base).await;
        sqlx::query!(
            "UPDATE plugins.deliveries SET attempts = $1",
            MAX_ATTEMPTS - 1
        )
        .execute(&pool)
        .await
        .unwrap();

        deliverer().once(&pool).await.unwrap();

        let dead = state(&pool).await;
        assert!(dead.failed && !dead.delivered);
        assert_eq!(dead.attempts, MAX_ATTEMPTS);
        make_due(&pool).await;
        assert_eq!(deliverer().once(&pool).await.unwrap(), 0);
        assert_eq!(plugin.requests(), 1);
    })
    .await;
}

#[tokio::test]
async fn a_disabled_plugin_is_sent_nothing_and_gets_what_it_missed_when_enabled_again() {
    db_test(async |pool| {
        let plugin = FakePlugin::start(202).await;
        owed(&pool, &plugin.base).await;
        let mut conn = pool.acquire().await.unwrap();
        set_status(&mut conn, "hello", InstallStatus::Disabled)
            .await
            .unwrap();

        assert_eq!(deliverer().once(&pool).await.unwrap(), 0);
        assert_eq!(plugin.requests(), 0);
        assert!(!state(&pool).await.delivered);

        set_status(&mut conn, "hello", InstallStatus::Enabled)
            .await
            .unwrap();
        assert_eq!(deliverer().once(&pool).await.unwrap(), 1);
        assert!(state(&pool).await.delivered);
    })
    .await;
}

#[tokio::test]
async fn a_plugin_that_cannot_be_reached_is_a_failed_attempt_and_not_an_error() {
    db_test(async |pool| {
        // Port 9 (discard) refuses connections on a test machine.
        owed(&pool, "http://127.0.0.1:9").await;

        assert_eq!(deliverer().once(&pool).await.unwrap(), 1);

        let failed = state(&pool).await;
        assert!(!failed.delivered && failed.attempts == 1);
        let error = failed.last_error.unwrap();
        assert!(!error.is_empty() && !error.contains("127.0.0.1"), "{error}");
    })
    .await;
}
