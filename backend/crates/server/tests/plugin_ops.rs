//! ADR-0079: what an administrator has to run a plugin that is in use. A token can be replaced, each
//! token has an allowance of calls, what a plugin calls is logged without its content, and what it
//! changes is in the audit log.

mod common;

use ::common::Role;
use access::{CourseScope, Scope};
use common::{Account, Env, db_test, spawn_app, test_settings};
use plugins::{Installed, parse_manifest};
use serde_json::{Value, json};
use server::app::AppState;
use sqlx::PgPool;
use uuid::Uuid;
use vumlo_config::{Secret, Settings};

fn settings() -> Settings {
    let mut settings = test_settings();
    settings.plugins.secret_key = Secret::new("0123456789abcdef0123456789abcdef");
    settings
}

struct World {
    env: Env,
    state: AppState,
    teacher: Account,
    admin: Account,
    assignment: Uuid,
}

async fn world(pool: PgPool, settings: Settings) -> World {
    let (base, state) = spawn_app(pool.clone(), settings).await;
    let env = Env {
        http: common::Http::new(&base),
        pool,
        base,
    };
    let teacher = env.user("teacher@x.edu", Role::Teacher).await;
    let admin = env.user("admin@x.edu", Role::Admin).await;
    let student = env.user("student@x.edu", Role::Student).await;
    let (_, module) = env.course(&teacher, &[&student]).await;
    let assignment = env.assignment(&teacher, module, "10").await;
    World {
        env,
        state,
        teacher,
        admin,
        assignment,
    }
}

impl World {
    async fn install(&self, id: &str, scopes: &[Scope]) -> Installed {
        let manifest = parse_manifest(
            json!({"id": id, "version": "1", "api": "1", "scopes": Scope::ALL.map(Scope::as_str)})
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
            scopes,
            &CourseScope::All,
        )
        .await
        .unwrap()
    }

    async fn call(
        &self,
        token: &str,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> common::Resp {
        let http = common::Http::new(&self.env.base);
        let url = format!("/plugin-api{path}");
        let request = match method {
            "PUT" => http.put(&url),
            "DELETE" => http.delete(&url),
            _ => http.get(&url),
        }
        .bearer(token);
        match body {
            Some(body) => request.json(&body).send().await,
            None => request.send().await,
        }
    }

    async fn lines(&self) -> Vec<(String, String, i16)> {
        sqlx::query_as("SELECT method, route, status FROM plugins.api_calls ORDER BY at, id")
            .fetch_all(&self.env.pool)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn a_new_token_replaces_the_old_one_at_once_and_the_secret_stays() {
    db_test(async |pool| {
        let w = world(pool, settings()).await;
        let installed = w.install("p", &[Scope::AssignmentsRead]).await;
        let old = installed.token.expose().to_owned();
        assert_eq!(w.call(&old, "GET", "/assignments", None).await.status, 200);

        let mut conn = w.state.db.acquire().await.unwrap();
        let fresh = plugins::rotate_token(&mut conn, "p").await.unwrap();
        drop(conn);
        assert_ne!(fresh.expose(), old);
        w.call(&old, "GET", "/assignments", None).await.problem(401);
        assert_eq!(
            w.call(fresh.expose(), "GET", "/assignments", None)
                .await
                .status,
            200
        );
        // Only the hash of the new one is kept, and the webhook secret did not change.
        let row: String =
            sqlx::query_scalar("SELECT row_to_json(t)::text FROM plugins.plugin_installs t")
                .fetch_one(&w.env.pool)
                .await
                .unwrap();
        assert!(!row.contains(fresh.expose()) && !row.contains(&installed.secret));
        let mut conn = w.state.db.acquire().await.unwrap();
        let now = plugins::get(&mut conn, "p").await.unwrap();
        let key = w.state.plugins.key.as_deref().unwrap();
        assert_eq!(
            plugins::derive_secret(key, now.id, now.secret_version).unwrap(),
            installed.secret
        );
        assert!(matches!(
            plugins::rotate_token(&mut conn, "nobody").await,
            Err(plugins::PluginsError::NotFound(_))
        ));
    })
    .await;
}

#[tokio::test]
async fn each_token_has_an_allowance_of_calls_and_a_flood_of_garbage_has_one_too() {
    db_test(async |pool| {
        let mut config = settings();
        config.plugins.api_burst = 3;
        config.plugins.api_requests_per_second = 1;
        let w = world(pool, config).await;
        let busy = w.install("busy", &[Scope::AssignmentsRead]).await;
        let quiet = w.install("quiet", &[Scope::AssignmentsRead]).await;

        for _ in 0..3 {
            assert_eq!(
                w.call(busy.token.expose(), "GET", "/assignments", None)
                    .await
                    .status,
                200
            );
        }
        // The fourth is turned away, with when to come back, as a problem document.
        let refused = w
            .call(busy.token.expose(), "GET", "/assignments", None)
            .await;
        refused.problem(429);
        assert!(!refused.header("retry-after").is_empty());
        // Another plugin has its own allowance.
        assert_eq!(
            w.call(quiet.token.expose(), "GET", "/assignments", None)
                .await
                .status,
            200
        );
        // A turned-away call is not logged; the three that went through are.
        let lines = w.lines().await;
        assert_eq!(lines.iter().filter(|l| l.2 == 429).count(), 0);
        assert_eq!(lines.len(), 4);

        // Calls with no token, or tokens nobody issued, share one allowance: they are 401 until it
        // is used up, then 429, and the real tokens are not touched by them.
        let mut seen = Vec::new();
        for n in 0..6 {
            let fake = format!("vplg_{}", format!("{n}").repeat(43)[..43].to_owned());
            seen.push(w.call(&fake, "GET", "/assignments", None).await.status);
        }
        assert_eq!(&seen[..3], [401, 401, 401], "{seen:?}");
        assert!(seen[3..].contains(&429), "{seen:?}");
        // The allowance comes back with time: a token that waited can call again.
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        assert_eq!(
            w.call(busy.token.expose(), "GET", "/assignments", None)
                .await
                .status,
            200
        );
        // What people do through the core is not counted against any plugin.
        for _ in 0..6 {
            assert_eq!(
                w.env
                    .call(
                        "GET",
                        &w.teacher.token,
                        "/plugin-slots?slot=assignment.tab",
                        None
                    )
                    .await
                    .status,
                200
            );
        }
    })
    .await;
}

#[tokio::test]
async fn a_call_leaves_a_line_with_the_route_and_the_answer_and_nothing_of_its_content() {
    db_test(async |pool| {
        let w = world(pool, settings()).await;
        let p = w
            .install("p", &[Scope::AssignmentsRead, Scope::ReportsWrite])
            .await;
        let q = w.install("q", &[Scope::AssignmentsRead]).await;
        let token = p.token.expose();

        w.call(
            token,
            "GET",
            &format!("/assignments/{}", w.assignment),
            None,
        )
        .await;
        w.call(
            token,
            "GET",
            &format!("/assignments/{}", Uuid::now_v7()),
            None,
        )
        .await;
        w.call(token, "GET", "/grades", None).await; // no scope
        w.call(
            token,
            "PUT",
            &format!("/assignments/{}/reports/secret-key-name", w.assignment),
            Some(json!({"summary": "PRIVATE SUMMARY", "data": {"x": "PRIVATE DATA"}})),
        )
        .await;
        w.call(
            q.token.expose(),
            "GET",
            "/assignments?course_id=00000000-0000-0000-0000-000000000000",
            None,
        )
        .await;
        // No token, a wrong token, a malformed token: no plugin to put a line under.
        w.call("junk", "GET", "/assignments", None).await;
        w.call(
            &format!("vplg_{}", "A".repeat(43)),
            "GET",
            "/assignments",
            None,
        )
        .await;

        let lines = w.lines().await;
        let route = |l: &(String, String, i16)| (l.0.clone(), l.1.clone(), l.2);
        assert_eq!(
            lines.iter().map(route).collect::<Vec<_>>(),
            vec![
                (
                    "GET".into(),
                    "/api/v1/plugin-api/assignments/{id}".into(),
                    200
                ),
                (
                    "GET".into(),
                    "/api/v1/plugin-api/assignments/{id}".into(),
                    404
                ),
                ("GET".into(), "/api/v1/plugin-api/grades".into(), 403),
                (
                    "PUT".into(),
                    "/api/v1/plugin-api/assignments/{id}/reports/{key}".into(),
                    204
                ),
                ("GET".into(), "/api/v1/plugin-api/assignments".into(), 404),
            ]
        );
        // Nothing of the call but its shape: no id, no key, no query, no content.
        let all: String = sqlx::query_scalar(
            "SELECT coalesce(string_agg(row_to_json(t)::text, ' '), '') FROM plugins.api_calls t",
        )
        .fetch_one(&w.env.pool)
        .await
        .unwrap();
        for never in [
            w.assignment.to_string(),
            "secret-key-name".into(),
            "PRIVATE".into(),
            "course_id".into(),
        ] {
            assert!(!all.contains(&never), "{never} is in the log: {all}");
        }
        // The lines belong to their plugin and go with it.
        let owners: Vec<(String, i64)> = sqlx::query_as(
            "SELECT i.plugin_id, count(*) FROM plugins.api_calls c
             JOIN plugins.plugin_installs i ON i.id = c.install_id GROUP BY 1 ORDER BY 1",
        )
        .fetch_all(&w.env.pool)
        .await
        .unwrap();
        assert_eq!(owners, [("p".to_owned(), 4), ("q".to_owned(), 1)]);
        let mut conn = w.state.db.acquire().await.unwrap();
        plugins::remove(&mut conn, "p").await.unwrap();
        assert_eq!(w.lines().await.len(), 1);
    })
    .await;
}

#[tokio::test]
async fn the_counts_and_the_newest_calls_are_read_back_and_old_lines_are_deleted() {
    db_test(async |pool| {
        let w = world(pool, settings()).await;
        let p = w.install("p", &[Scope::AssignmentsRead]).await;
        for _ in 0..3 {
            w.call(p.token.expose(), "GET", "/assignments", None).await;
        }
        w.call(p.token.expose(), "GET", "/grades", None).await;
        let mut conn = w.state.db.acquire().await.unwrap();
        let since = chrono::Utc::now() - chrono::TimeDelta::hours(1);
        let report = plugins::calls(&mut conn, "p", since, 2).await.unwrap();
        assert_eq!(report.recent.len(), 2, "the limit");
        assert_eq!(report.recent[0].route, "/api/v1/plugin-api/grades", "newest first");
        let by: Vec<(String, i16, i64)> = report
            .counts
            .iter()
            .map(|c| (c.route.clone(), c.status, c.calls))
            .collect();
        assert_eq!(
            by,
            [
                ("/api/v1/plugin-api/assignments".into(), 200, 3),
                ("/api/v1/plugin-api/grades".into(), 403, 1),
            ]
        );
        // A look further back than the lines go finds nothing.
        let future = chrono::Utc::now() + chrono::TimeDelta::hours(1);
        assert!(plugins::calls(&mut conn, "p", future, 5).await.unwrap().counts.is_empty());
        assert!(matches!(
            plugins::calls(&mut conn, "nobody", since, 5).await,
            Err(plugins::PluginsError::NotFound(_))
        ));

        // The delivery worker deletes what is older than the retention, and only that.
        sqlx::query("UPDATE plugins.api_calls SET at = now() - interval '31 days' WHERE route LIKE '%grades'")
            .execute(&w.env.pool)
            .await
            .unwrap();
        let deliverer = w.state.plugin_deliverer().unwrap().unwrap();
        assert_eq!(deliverer.prune(&w.state.db).await.unwrap(), 1);
        assert_eq!(w.lines().await.len(), 3);
    })
    .await;
}

#[tokio::test]
async fn what_a_plugin_changes_is_in_the_audit_log_as_the_plugin_and_never_as_a_person() {
    db_test(async |pool| {
        let w = world(pool, settings()).await;
        let p = w
            .install("p", &[Scope::ReportsWrite, Scope::AssignmentsRead])
            .await;
        let path = format!("/assignments/{}/reports/totals", w.assignment);
        let put = w
            .call(
                p.token.expose(),
                "PUT",
                &path,
                Some(json!({"summary": "PRIVATE SUMMARY", "data": {}})),
            )
            .await;
        assert_eq!(put.status, 204);
        // A refused write leaves no audit row; a read leaves none either.
        w.call(
            p.token.expose(),
            "PUT",
            &format!("/assignments/{}/reports/Bad Key", w.assignment),
            Some(json!({"summary": "x", "data": {}})),
        )
        .await;
        w.call(p.token.expose(), "GET", "/assignments", None).await;
        assert_eq!(
            w.call(p.token.expose(), "DELETE", &path, None).await.status,
            204
        );

        let rows: Vec<(String, Option<Uuid>, String, String, Uuid, Value)> = sqlx::query_as(
            "SELECT source, actor_id, action, resource_type, resource_id, meta
             FROM events.audit_log WHERE source = 'plugin' ORDER BY at, id",
        )
        .fetch_all(&w.env.pool)
        .await
        .unwrap();
        assert_eq!(rows.len(), 2, "{rows:?}");
        let (put_row, delete_row) = (&rows[0], &rows[1]);
        assert_eq!(put_row.1, None, "a plugin is not a person");
        assert_eq!(
            (put_row.2.as_str(), put_row.3.as_str(), put_row.4),
            ("plugin.report.put", "assignment", w.assignment)
        );
        assert_eq!(delete_row.2, "plugin.report.delete");
        assert_eq!(put_row.5["plugin_id"], "p");
        assert_eq!(put_row.5["key"], "totals");
        assert_eq!(put_row.5["install_id"], json!(p.install.id.to_string()));
        assert!(!put_row.5.to_string().contains("PRIVATE"));

        // The administrators' audit report shows it, and says who it was not.
        let page = w
            .env
            .ok("GET", &w.admin.token, "/admin/audit?limit=50", None, 200)
            .await;
        let items = page["items"].as_array().unwrap();
        let ours: Vec<_> = items.iter().filter(|i| i["source"] == "plugin").collect();
        assert_eq!(ours.len(), 2, "{page}");
        assert!(ours.iter().all(|i| i["actor"].is_null()));
        assert_eq!(ours[0]["meta"]["plugin_id"], "p");
    })
    .await;
}
