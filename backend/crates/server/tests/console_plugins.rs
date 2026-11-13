//! Administering plugins from the console (ADR-0095): the list, the settings the manifests declare,
//! the grants, the switch and the call log. Nothing here shows a secret.

mod common;

use ::common::Role;
use access::{CourseScope, Scope};
use common::{Account, Env, db_test, spawn_app, test_settings};
use plugins::parse_manifest;
use serde_json::{Value, json};
use server::app::AppState;
use sqlx::PgPool;
use vumlo_config::Secret;

struct World {
    env: Env,
    state: AppState,
    admin: Account,
    app_admin: Account,
    teacher: Account,
}

async fn world(pool: PgPool) -> World {
    let mut settings = test_settings();
    settings.plugins.secret_key = Secret::new("0123456789abcdef0123456789abcdef");
    let (base, state) = spawn_app(pool.clone(), settings).await;
    let env = Env {
        http: common::Http::new(&base),
        pool,
        base,
    };
    World {
        admin: env.user("admin@x.edu", Role::Admin).await,
        app_admin: env.app_user("app-admin@x.edu", Role::Admin).await,
        teacher: env.user("teacher@x.edu", Role::Teacher).await,
        env,
        state,
    }
}

impl World {
    /// An install named `id` with a secret, a pick, a number and a setting that belongs to a course.
    async fn install(&self, id: &str) {
        let manifest = parse_manifest(
            json!({
                "id": id, "version": "1", "api": "1",
                "scopes": ["settings:read", "submissions:read", "mail:send"],
                "settings": {
                    "api_token": {"type": "string", "secret": true, "description": "The service token"},
                    "mode": {"type": "string", "enum": ["fast", "slow"], "default": "fast"},
                    "limit": {"type": "number", "minimum": 1, "maximum": 10},
                    "note": {"type": "string", "scope": "course"}
                }
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        let key = self.state.plugins.key.as_deref().expect("a plugin key");
        let mut conn = self.state.db.acquire().await.unwrap();
        plugins::install(
            &mut conn,
            key,
            "http://127.0.0.1:9",
            &manifest,
            &[Scope::SettingsRead],
            &CourseScope::All,
        )
        .await
        .unwrap();
    }

    async fn call(
        &self,
        who: &Account,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> common::Resp {
        self.env.call(method, &who.token, path, body).await
    }
}

/// Every route of the plugin administration, with a body that is valid for it.
fn routes() -> Vec<(&'static str, String, Option<Value>)> {
    vec![
        ("GET", "/admin/plugins".into(), None),
        ("GET", "/admin/plugins/ocr".into(), None),
        (
            "PUT",
            "/admin/plugins/ocr/status".into(),
            Some(json!({"status": "enabled"})),
        ),
        (
            "PUT",
            "/admin/plugins/ocr/grants".into(),
            Some(json!({"scopes": ["settings:read"], "all_courses": true, "course_ids": []})),
        ),
        ("GET", "/admin/plugins/ocr/settings".into(), None),
        (
            "PUT",
            "/admin/plugins/ocr/settings/mode".into(),
            Some(json!({"value": "slow"})),
        ),
        ("DELETE", "/admin/plugins/ocr/settings/mode".into(), None),
        ("GET", "/admin/plugins/ocr/calls".into(), None),
    ]
}

#[tokio::test]
async fn an_admin_app_session_is_refused_the_plugin_routes_and_the_console_is_not() {
    db_test(async |pool| {
        let w = world(pool).await;
        w.install("ocr").await;
        for (method, path, body) in routes() {
            for (who, name) in [(&w.app_admin, "app admin"), (&w.teacher, "teacher")] {
                let status = w.call(who, method, &path, body.clone()).await.status;
                assert_eq!(status, 403, "{name}: {method} {path}");
            }
            let status = w.call(&w.admin, method, &path, body.clone()).await.status;
            assert!(
                ![401, 403].contains(&status) && status < 500,
                "console: {method} {path} -> {status}"
            );
        }
        assert_eq!(w.env.http.get("/admin/plugins").send().await.status, 401);
    })
    .await;
}

#[tokio::test]
async fn the_console_lists_installs_with_status_scopes_and_what_needs_setup() {
    db_test(async |pool| {
        let w = world(pool).await;
        w.install("ocr").await;
        let list = w.call(&w.admin, "GET", "/admin/plugins", None).await.json();
        let item = &list["items"][0];
        assert_eq!(item["plugin_id"], "ocr");
        assert_eq!(item["status"], "enabled");
        assert_eq!(item["scopes"], json!(["settings:read"]));
        assert_eq!(item["requested_scopes"].as_array().unwrap().len(), 3);
        assert_eq!(item["all_courses"], true);
        // `api_token` and `limit` have no value and no default; `mode` has a default; `note` is the
        // teacher's.
        let mut needs: Vec<String> = serde_json::from_value(item["needs_setup"].clone()).unwrap();
        needs.sort();
        assert_eq!(needs, ["api_token", "limit"]);
    })
    .await;
}

#[tokio::test]
async fn a_secret_setting_is_accepted_and_never_returned() {
    db_test(async |pool| {
        let w = world(pool.clone()).await;
        w.install("ocr").await;
        let put = w
            .call(
                &w.admin,
                "PUT",
                "/admin/plugins/ocr/settings/api_token",
                Some(json!({"value": "sk-sentinel-123"})),
            )
            .await;
        assert_eq!(put.status, 204, "{}", String::from_utf8_lossy(&put.body));
        assert!(!String::from_utf8_lossy(&put.body).contains("sentinel"));
        let settings = w
            .call(&w.admin, "GET", "/admin/plugins/ocr/settings", None)
            .await;
        assert!(!String::from_utf8_lossy(&settings.body).contains("sentinel"));
        let view = settings.json()["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["key"] == "api_token")
            .cloned()
            .unwrap();
        assert_eq!(view["is_set"], true);
        assert_eq!(view["secret"], true);
        assert!(view.get("value").is_none_or(Value::is_null), "{view}");
        // The audit entry names the key, never the value; the actor is the administrator.
        let (meta, actor): (Value, Option<uuid::Uuid>) = sqlx::query_as(
            "SELECT meta, actor_id FROM events.audit_log WHERE action = 'plugin.setting.set'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(meta["key"], "api_token");
        assert!(!meta.to_string().contains("sentinel"));
        assert_eq!(actor, Some(w.admin.id));
    })
    .await;
}

#[tokio::test]
async fn a_refused_value_is_a_422_that_names_the_setting_and_nothing_is_stored() {
    db_test(async |pool| {
        let w = world(pool.clone()).await;
        w.install("ocr").await;
        let too_big = w
            .call(
                &w.admin,
                "PUT",
                "/admin/plugins/ocr/settings/limit",
                Some(json!({"value": "99"})),
            )
            .await;
        let problem = too_big.problem(422);
        assert!(
            problem.errors.iter().any(|e| e.field == "value"),
            "{problem:?}"
        );
        let not_a_pick = w
            .call(
                &w.admin,
                "PUT",
                "/admin/plugins/ocr/settings/mode",
                Some(json!({"value": "medium"})),
            )
            .await;
        assert_eq!(not_a_pick.status, 422);
        // A setting of a course is the teacher's; an unknown one does not exist.
        let course = w
            .call(
                &w.admin,
                "PUT",
                "/admin/plugins/ocr/settings/note",
                Some(json!({"value": "x"})),
            )
            .await;
        assert_eq!(course.status, 422);
        let unknown = w
            .call(
                &w.admin,
                "PUT",
                "/admin/plugins/ocr/settings/nope",
                Some(json!({"value": "x"})),
            )
            .await;
        assert_eq!(unknown.status, 404);
        let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM plugins.settings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored, 0);
        let audited: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events.audit_log WHERE action LIKE 'plugin.%'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(audited, 0);
    })
    .await;
}

#[tokio::test]
async fn unsetting_brings_the_default_back_and_the_second_time_changes_nothing() {
    db_test(async |pool| {
        let w = world(pool.clone()).await;
        w.install("ocr").await;
        let path = "/admin/plugins/ocr/settings/mode";
        assert_eq!(
            w.call(&w.admin, "PUT", path, Some(json!({"value": "slow"})))
                .await
                .status,
            204
        );
        assert_eq!(w.call(&w.admin, "DELETE", path, None).await.status, 204);
        let again = w.call(&w.admin, "DELETE", path, None).await;
        assert_eq!(again.status, 204);
        let audited: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events.audit_log WHERE action = 'plugin.setting.unset'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(audited, 1, "only the real unset is audited");
        let view = w
            .call(&w.admin, "GET", "/admin/plugins/ocr/settings", None)
            .await
            .json();
        let mode = view["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["key"] == "mode")
            .cloned()
            .unwrap();
        assert_eq!(
            (mode["is_set"].clone(), mode["value"].clone()),
            (json!(false), json!("fast"))
        );
    })
    .await;
}

#[tokio::test]
async fn switching_a_plugin_off_and_on_is_audited_with_the_actor_and_repeating_it_changes_nothing()
{
    db_test(async |pool| {
        let w = world(pool.clone()).await;
        w.install("ocr").await;
        let off = w
            .call(
                &w.admin,
                "PUT",
                "/admin/plugins/ocr/status",
                Some(json!({"status": "disabled"})),
            )
            .await;
        assert_eq!(off.status, 200);
        assert_eq!(off.json()["status"], "disabled");
        let on = w
            .call(
                &w.admin,
                "PUT",
                "/admin/plugins/ocr/status",
                Some(json!({"status": "enabled"})),
            )
            .await;
        assert_eq!(on.json()["status"], "enabled");
        // Asking for the state it has is not a change: no second entry.
        w.call(
            &w.admin,
            "PUT",
            "/admin/plugins/ocr/status",
            Some(json!({"status": "enabled"})),
        )
        .await;
        let actors: Vec<Option<uuid::Uuid>> = sqlx::query_scalar(
            "SELECT actor_id FROM events.audit_log WHERE action LIKE 'plugin.%'",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(actors, [Some(w.admin.id), Some(w.admin.id)]);
        // The queued mail of a plugin that is switched off is cancelled by the same function the
        // CLI calls (`plugins::set_status`), which `tests/plugin_mail.rs` covers.
        let actions: Vec<String> = sqlx::query_scalar(
            "SELECT action FROM events.audit_log WHERE action LIKE 'plugin.%' ORDER BY at, id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(actions, ["plugin.disable", "plugin.enable"]);
    })
    .await;
}

#[tokio::test]
async fn grants_stay_within_what_the_plugin_asked_and_the_change_is_audited() {
    db_test(async |pool| {
        let w = world(pool.clone()).await;
        w.install("ocr").await;
        let ok = w
            .call(
                &w.admin,
                "PUT",
                "/admin/plugins/ocr/grants",
                Some(json!({"scopes": ["settings:read", "submissions:read"], "all_courses": true, "course_ids": []})),
            )
            .await;
        assert_eq!(ok.status, 200, "{}", String::from_utf8_lossy(&ok.body));
        let mut scopes: Vec<String> = serde_json::from_value(ok.json()["scopes"].clone()).unwrap();
        scopes.sort();
        assert_eq!(scopes, ["settings:read", "submissions:read"]);
        let over = w
            .call(
                &w.admin,
                "PUT",
                "/admin/plugins/ocr/grants",
                Some(json!({"scopes": ["grades:read"], "all_courses": true, "course_ids": []})),
            )
            .await;
        let problem = over.problem(422);
        assert!(problem.errors.iter().any(|e| e.field == "scopes"), "{problem:?}");
        let audited: i64 = sqlx::query_scalar("SELECT count(*) FROM events.audit_log WHERE action = 'plugin.grant'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(audited, 1);
    })
    .await;
}

#[tokio::test]
async fn the_calls_report_counts_by_route_and_answer() {
    db_test(async |pool| {
        let w = world(pool.clone()).await;
        w.install("ocr").await;
        let install: uuid::Uuid =
            sqlx::query_scalar("SELECT id FROM plugins.plugin_installs WHERE plugin_id = 'ocr'")
                .fetch_one(&pool)
                .await
                .unwrap();
        for status in [200i16, 200, 403] {
            sqlx::query(
                "INSERT INTO plugins.api_calls (id, install_id, method, route, status, duration_ms)
                 VALUES ($3, $1, 'GET', '/plugin-api/settings', $2, 5)",
            )
            .bind(install)
            .bind(status)
            .bind(uuid::Uuid::now_v7())
            .execute(&pool)
            .await
            .unwrap();
        }
        let report = w
            .call(&w.admin, "GET", "/admin/plugins/ocr/calls?limit=10", None)
            .await
            .json();
        assert_eq!(report["recent"].as_array().unwrap().len(), 3);
        let counts = report["counts"].as_array().unwrap();
        let ok = counts.iter().find(|c| c["status"] == 200).unwrap();
        assert_eq!(ok["calls"], 2);
        assert_eq!(ok["route"], "/plugin-api/settings");
    })
    .await;
}
