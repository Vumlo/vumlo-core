//! The mail queue and journal in the console (ADR-0095): what waits, what was sent, what died, and
//! putting dead mail back. The page names people by id and never shows an address.

mod common;

use ::common::Role;
use common::{Account, Env, db_test};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

struct World {
    env: Env,
    admin: Account,
    app_admin: Account,
    teacher: Account,
}

async fn world(pool: PgPool) -> World {
    let env = Env::new(pool).await;
    World {
        admin: env.user("admin@x.edu", Role::Admin).await,
        app_admin: env.app_user("app-admin@x.edu", Role::Admin).await,
        teacher: env.user("teacher@x.edu", Role::Teacher).await,
        env,
    }
}

impl World {
    async fn call(
        &self,
        who: &Account,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> common::Resp {
        self.env.call(method, &who.token, path, body).await
    }

    /// One message of `plugin` in `state`, to an address nobody may see.
    async fn message(&self, plugin: Option<&str>, kind: &str, state: &str, erased: bool) -> Uuid {
        let batch = Uuid::now_v7();
        sqlx::query("INSERT INTO mail.batches (id, plugin_id, kind, subject) VALUES ($1, $2, $3, 'Sentinel subject')")
            .bind(batch)
            .bind(plugin)
            .bind(kind)
            .execute(&self.env.pool)
            .await
            .unwrap();
        let id = Uuid::now_v7();
        let finished = state != "pending";
        sqlx::query(
            "INSERT INTO mail.messages (id, batch_id, user_id, plugin_id, kind, to_address, state, last_error, finished_at)
             VALUES ($1, $2, $3, $9, $4, $5, $6, $7, CASE WHEN $8 THEN now() END)",
        )
        .bind(id)
        .bind(batch)
        .bind(self.teacher.id)
        .bind(kind)
        .bind(if erased { None } else { Some("sentinel-address@x.edu") })
        .bind(state)
        .bind((state == "dead").then_some("smtp 451"))
        .bind(finished)
        .bind(plugin)
        .execute(&self.env.pool)
        .await
        .unwrap();
        id
    }
}

fn routes() -> Vec<(&'static str, &'static str, Option<Value>)> {
    vec![
        ("GET", "/admin/mail/status", None),
        ("GET", "/admin/mail/messages", None),
        ("POST", "/admin/mail/retry", Some(json!({}))),
    ]
}

#[tokio::test]
async fn an_admin_app_session_is_refused_the_mail_routes_and_the_console_is_not() {
    db_test(async |pool| {
        let w = world(pool).await;
        for (method, path, body) in routes() {
            for (who, name) in [(&w.app_admin, "app admin"), (&w.teacher, "teacher")] {
                let status = w.call(who, method, path, body.clone()).await.status;
                assert_eq!(status, 403, "{name}: {method} {path}");
            }
            let status = w.call(&w.admin, method, path, body.clone()).await.status;
            assert!(
                ![401, 403].contains(&status) && status < 500,
                "console: {method} {path} -> {status}"
            );
            let anonymous = match method {
                "POST" => w.env.http.post(path).json(&json!({})).send().await,
                _ => w.env.http.get(path).send().await,
            };
            assert_eq!(anonymous.status, 401, "{path}");
        }
    })
    .await;
}

#[tokio::test]
async fn status_reports_what_waits_was_sent_and_died() {
    db_test(async |pool| {
        let w = world(pool).await;
        w.message(Some("ocr"), "deadline", "pending", false).await;
        w.message(Some("ocr"), "deadline", "sent", true).await;
        w.message(Some("ocr"), "deadline", "sent", true).await;
        w.message(Some("ocr"), "grade", "dead", false).await;
        let status = w
            .call(&w.admin, "GET", "/admin/mail/status", None)
            .await
            .json();
        assert_eq!(status["configured"], false, "the test server has no [mail]");
        assert_eq!(status["pending"], 1);
        assert_eq!(status["sent_24h"], 2);
        assert_eq!(status["dead_24h"], 1);
        assert_eq!(status["dead_now"], 1);
        let by = &status["dead_by"][0];
        assert_eq!(
            (
                by["plugin_id"].clone(),
                by["kind"].clone(),
                by["dead"].clone()
            ),
            (json!("ocr"), json!("grade"), json!(1))
        );
    })
    .await;
}

#[tokio::test]
async fn the_journal_lists_ids_plugin_kind_and_state_and_no_address() {
    db_test(async |pool| {
        let w = world(pool).await;
        let dead = w.message(Some("ocr"), "grade", "dead", false).await;
        w.message(None, "account", "sent", true).await;
        let all = w.call(&w.admin, "GET", "/admin/mail/messages", None).await;
        let text = String::from_utf8_lossy(&all.body).into_owned();
        assert!(!text.contains("sentinel-address"), "{text}");
        let items = all.json()["items"].as_array().unwrap().clone();
        assert_eq!(items.len(), 2);
        let mine = items.iter().find(|m| m["id"] == json!(dead)).unwrap();
        assert_eq!(mine["state"], "dead");
        assert_eq!(mine["plugin_id"], "ocr");
        assert_eq!(mine["kind"], "grade");
        assert_eq!(mine["user_id"], json!(w.teacher.id));
        assert_eq!(mine["last_error"], "smtp 451");

        let only_dead = w
            .call(&w.admin, "GET", "/admin/mail/messages?state=dead", None)
            .await
            .json();
        assert_eq!(only_dead["items"].as_array().unwrap().len(), 1);
        let only_plugin = w
            .call(
                &w.admin,
                "GET",
                "/admin/mail/messages?plugin_id=ocr&kind=grade",
                None,
            )
            .await
            .json();
        assert_eq!(only_plugin["items"].as_array().unwrap().len(), 1);
        let bad = w
            .call(&w.admin, "GET", "/admin/mail/messages?state=lost", None)
            .await;
        assert_eq!(bad.status, 422);
    })
    .await;
}

#[tokio::test]
async fn retry_requeues_dead_messages_leaves_erased_ones_and_audits_the_count() {
    db_test(async |pool| {
        let w = world(pool.clone()).await;
        let live = w.message(Some("ocr"), "grade", "dead", false).await;
        let erased = w.message(Some("ocr"), "grade", "dead", true).await;
        let other = w.message(Some("other"), "grade", "dead", false).await;

        let answer = w
            .call(
                &w.admin,
                "POST",
                "/admin/mail/retry",
                Some(json!({"plugin_id": "ocr"})),
            )
            .await;
        assert_eq!(
            answer.status,
            200,
            "{}",
            String::from_utf8_lossy(&answer.body)
        );
        assert_eq!(answer.json(), json!({"requeued": 1, "erased": 1}));
        let state = |id: Uuid| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, String>("SELECT state FROM mail.messages WHERE id = $1")
                    .bind(id)
                    .fetch_one(&pool)
                    .await
                    .unwrap()
            }
        };
        assert_eq!(state(live).await, "pending");
        assert_eq!(state(erased).await, "dead");
        assert_eq!(
            state(other).await,
            "dead",
            "another plugin's mail is not touched"
        );

        let (meta, actor): (Value, Option<Uuid>) = sqlx::query_as(
            "SELECT meta, actor_id FROM events.audit_log WHERE action = 'mail.retry'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(meta["count"], 1);
        assert_eq!(meta["plugin"], "ocr");
        assert_eq!(actor, Some(w.admin.id));

        // Nothing left to put back: nothing is audited again.
        let again = w
            .call(
                &w.admin,
                "POST",
                "/admin/mail/retry",
                Some(json!({"plugin_id": "ocr"})),
            )
            .await;
        assert_eq!(again.json(), json!({"requeued": 0, "erased": 1}));
        let audited: i64 =
            sqlx::query_scalar("SELECT count(*) FROM events.audit_log WHERE action = 'mail.retry'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(audited, 1);

        // One message by its id.
        let one = w
            .call(
                &w.admin,
                "POST",
                "/admin/mail/retry",
                Some(json!({"id": other})),
            )
            .await;
        assert_eq!(one.json(), json!({"requeued": 1, "erased": 0}));
    })
    .await;
}
