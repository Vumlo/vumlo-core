//! Plugins (ADR-0066), from an administrator's install to a report on a submission, against the
//! real server and the real example plugin (`examples/hello-plugin/plugin.py`, Python 3 standard
//! library; these tests need `python3`).
//!
//! - an install approves scopes, and the core keeps only the hash of the token;
//! - a signed event is delivered and the report appears for staff;
//! - the plugin refuses a bad signature and a stale timestamp;
//! - a call without the scope is `403`, a draft is `404` and a wrong token is `401`;
//! - disabling the plugin stops its events and its token.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use ::common::Role;
use access::Scope;
use common::{Account, Env, db_test, essay, id_of, spawn_app, test_settings, wait_until};
use plugins::{InstallStatus, Installed, parse_manifest};
use serde_json::{Value, json};
use server::app::AppState;
use sqlx::PgPool;
use tokio::net::TcpStream;
use uuid::Uuid;
use vumlo_config::{Secret, Settings};

const KEY: &str = "0123456789abcdef0123456789abcdef";

fn settings() -> Settings {
    let mut settings = test_settings();
    settings.plugins.secret_key = Secret::new(KEY);
    settings
}

fn plugin_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("the server crate lies in backend/crates")
        .join("examples/hello-plugin/plugin.py")
}

/// The example plugin as a process. Killed when dropped.
struct Plugin {
    child: Child,
    port: u16,
}

impl Plugin {
    fn free_port() -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
        listener.local_addr().expect("its address").port()
    }

    /// Starts it on `port` with the credentials of an install, or without any (it then serves its
    /// manifest and refuses events), and waits until it listens.
    async fn start(port: u16, api: &str, credentials: Option<&Installed>) -> Self {
        let mut command = Command::new("python3");
        command
            .arg(plugin_script())
            .env("PORT", port.to_string())
            .env("VUMLO_API", api)
            .stdout(Stdio::null());
        if let Some(installed) = credentials {
            command
                .env("VUMLO_PLUGIN_TOKEN", installed.token.expose())
                .env("VUMLO_WEBHOOK_SECRET", &installed.secret);
        }
        let child = command.spawn().expect("run python3");
        let plugin = Self { child, port };
        wait_until(|| async { TcpStream::connect(("127.0.0.1", port)).await.is_ok() }).await;
        plugin
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct World {
    env: Env,
    state: AppState,
    teacher: Account,
    student: Account,
    assignment: Uuid,
    api: String,
}

async fn world(pool: PgPool) -> World {
    let (base, state) = spawn_app(pool.clone(), settings()).await;
    let env = Env {
        http: common::Http::new(&base),
        pool,
        base: base.clone(),
    };
    let teacher = env.user("teacher@x.edu", Role::Teacher).await;
    let student = env.user("student@x.edu", Role::Student).await;
    let (_, module) = env.course(&teacher, &[&student]).await;
    let assignment = env.assignment(&teacher, module, "10").await;
    World {
        api: format!("{base}/api/v1"),
        env,
        state,
        teacher,
        student,
        assignment,
    }
}

impl World {
    /// The student saves a draft and submits it. Returns the id of the attempt.
    async fn submit(&self, text: &str) -> Uuid {
        let path = format!("/assignments/{}/submission", self.assignment);
        self.env
            .ok(
                "PUT",
                &self.student.token,
                &format!("{path}/draft"),
                Some(json!({"content": essay(text)})),
                200,
            )
            .await;
        let attempt = self
            .env
            .ok(
                "POST",
                &self.student.token,
                &format!("{path}/submit"),
                None,
                201,
            )
            .await;
        id_of(&attempt)
    }

    /// What the core does by itself, once: the outbox owes the plugins, then the plugins are sent
    /// what they are owed. Returns how many deliveries it sent.
    async fn pump(&self) -> usize {
        events::process_once(&self.state.db, &self.state.event_handlers())
            .await
            .expect("the outbox");
        let deliverer = self
            .state
            .plugin_deliverer()
            .expect("the deliverer")
            .expect("a plugin key is set");
        deliverer.once(&self.state.db).await.expect("the delivery")
    }

    /// What the plugins reported about an attempt, as staff read it.
    async fn reports(&self, submission: Uuid) -> Vec<Value> {
        let path = format!("/submissions/{submission}/plugin-reports");
        let body = self
            .env
            .ok("GET", &self.teacher.token, &path, None, 200)
            .await;
        body["items"].as_array().expect("items").clone()
    }

    /// Installs the manifest of the example plugin at `url`, with these scopes granted, as
    /// `vumlo-cli plugin install` does.
    async fn install(&self, url: &str, granted: &[Scope]) -> Installed {
        let manifest = plugins::fetch_manifest(url).await.expect("the manifest");
        let key = self
            .state
            .plugins
            .key
            .as_deref()
            .expect("a plugin key is set");
        let mut conn = self.state.db.acquire().await.expect("a connection");
        plugins::install(&mut conn, key, url, &manifest, granted)
            .await
            .expect("the install")
    }

    /// The plugin API as the plugin calls it.
    fn plugin_call(&self, method: &str, path: &str, token: &str) -> common::Req {
        let http = common::Http::new(&self.env.base);
        let path = format!("/plugin-api{path}");
        let request = match method {
            "GET" => http.get(&path),
            _ => http.put(&path),
        };
        request.bearer(token)
    }
}

async fn wait_for_report(world: &World, submission: Uuid) -> Value {
    wait_until(|| async { !world.reports(submission).await.is_empty() }).await;
    world.reports(submission).await.remove(0)
}

#[tokio::test]
async fn a_signed_event_reaches_the_plugin_and_its_report_appears_for_staff() {
    db_test(async |pool| {
        let w = world(pool).await;
        let port = Plugin::free_port();

        // The administrator installs it: the core reads the manifest and the scopes are approved.
        let installed = {
            let bare = Plugin::start(port, &w.api, None).await;
            let installed = w
                .install(&bare.url(), &[Scope::SubmissionsRead, Scope::ReportsWrite])
                .await;
            assert_eq!(installed.install.plugin_id, "hello");
            assert_eq!(
                installed.install.requested_scopes,
                [Scope::SubmissionsRead, Scope::ReportsWrite]
            );
            installed
        };
        // Only the hash of the token is stored; neither it nor the secret is in the row.
        let row: String =
            sqlx::query_scalar("SELECT row_to_json(t)::text FROM plugins.plugin_installs t")
                .fetch_one(&w.env.pool)
                .await
                .unwrap();
        assert!(!row.contains(installed.token.expose()) && !row.contains(&installed.secret));

        // It is started again with what the install printed.
        let _plugin = Plugin::start(port, &w.api, Some(&installed)).await;
        let submission = w.submit("one two three four five").await;
        assert_eq!(w.pump().await, 1, "one delivery sent");

        let report = wait_for_report(&w, submission).await;
        assert_eq!(report["plugin_id"], "hello");
        assert_eq!(report["summary"], "5 words");
        assert_eq!(report["data"], json!({"words": 5}));

        // The outbox delivers at least once; a second pass owes the plugin nothing new.
        sqlx::query("UPDATE events.outbox SET processed_at = NULL, run_after = now()")
            .execute(&w.env.pool)
            .await
            .unwrap();
        events::process_once(&w.state.db, &w.state.event_handlers())
            .await
            .unwrap();
        let owed: i64 = sqlx::query_scalar("SELECT count(*) FROM plugins.deliveries")
            .fetch_one(&w.env.pool)
            .await
            .unwrap();
        assert_eq!(owed, 1);
        assert_eq!(w.reports(submission).await.len(), 1);
    })
    .await;
}

#[tokio::test]
async fn the_plugin_refuses_a_bad_signature_and_a_stale_timestamp() {
    db_test(async |pool| {
        let w = world(pool).await;
        let port = Plugin::free_port();
        let installed = {
            let bare = Plugin::start(port, &w.api, None).await;
            w.install(&bare.url(), &[Scope::SubmissionsRead, Scope::ReportsWrite])
                .await
        };
        let plugin = Plugin::start(port, &w.api, Some(&installed)).await;
        let body = json!({"id": Uuid::now_v7(), "type": "grade.published", "data": {}}).to_string();
        let now = chrono::Utc::now().timestamp();
        let post = async |timestamp: Option<i64>, signature: Option<String>, body: &str| {
            let mut request = reqwest::Client::new()
                .post(format!("{}/events", plugin.url()))
                .header("content-type", "application/json")
                .body(body.to_owned());
            if let Some(timestamp) = timestamp {
                request = request.header("x-vumlo-timestamp", timestamp.to_string());
            }
            if let Some(signature) = signature {
                request = request.header("x-vumlo-signature", signature);
            }
            request
                .send()
                .await
                .expect("the plugin answers")
                .status()
                .as_u16()
        };
        let sign = |timestamp: i64, body: &str| {
            plugins::sign(&installed.secret, timestamp, body.as_bytes()).unwrap()
        };

        // Fresh and signed with the secret of the install: taken.
        assert_eq!(post(Some(now), Some(sign(now, &body)), &body).await, 202);
        // The same request with one byte of the body changed, or another secret, or no signature.
        assert_eq!(
            post(
                Some(now),
                Some(sign(now, &body)),
                &body.replace("grade", "grAde")
            )
            .await,
            401
        );
        let other = plugins::sign("another-secret", now, body.as_bytes()).unwrap();
        assert_eq!(post(Some(now), Some(other), &body).await, 401);
        assert_eq!(post(Some(now), None, &body).await, 401);
        assert_eq!(post(None, Some(sign(now, &body)), &body).await, 401);
        // Signed correctly, but ten minutes ago, or ten minutes ahead: a replay is refused.
        for skewed in [now - 600, now + 600] {
            assert_eq!(
                post(Some(skewed), Some(sign(skewed, &body)), &body).await,
                401
            );
        }
        // A genuine request that is not an event is a plain 400, not a crash.
        let junk = "not json";
        assert_eq!(post(Some(now), Some(sign(now, junk)), junk).await, 400);
        // The shared vector: the plugin's code computes what the core computes.
        let check = Command::new("python3")
            .arg(plugin_script())
            .arg("--check-vector")
            .output()
            .expect("run python3");
        assert!(
            check.status.success(),
            "{}",
            String::from_utf8_lossy(&check.stderr)
        );
    })
    .await;
}

#[tokio::test]
async fn a_call_without_the_scope_is_refused_and_a_draft_is_not_there() {
    db_test(async |pool| {
        let w = world(pool).await;
        let reader = w
            .install_without_a_process("reader", &[Scope::SubmissionsRead])
            .await;
        let submission = w.submit("a b c").await;
        let report = json!({"summary": "x", "data": {}});

        // With `submissions:read` it reads the attempt; with no `reports:write` it cannot report.
        let read = w
            .plugin_call(
                "GET",
                &format!("/submissions/{submission}"),
                reader.token.expose(),
            )
            .send()
            .await;
        assert_eq!(read.status, 200, "{}", String::from_utf8_lossy(&read.body));
        let work = read.json();
        assert_eq!(work["id"], json!(submission));
        assert_eq!(work["student_id"], json!(w.student.id));
        assert_eq!(work["attempt"], 1);
        assert!(work.get("score").is_none() && work.get("draft").is_none());
        w.plugin_call(
            "PUT",
            &format!("/submissions/{submission}/report"),
            reader.token.expose(),
        )
        .json(&report)
        .send()
        .await
        .problem(403);
        assert!(w.reports(submission).await.is_empty());

        // A new draft on top of the attempt is not an attempt: no plugin sees it.
        w.env
            .ok(
                "PUT",
                &w.student.token,
                &format!("/assignments/{}/submission/draft", w.assignment),
                Some(json!({"content": essay("secret draft")})),
                200,
            )
            .await;
        let draft: Uuid =
            sqlx::query_scalar("SELECT id FROM assessment.submissions WHERE status = 'draft'")
                .fetch_one(&w.env.pool)
                .await
                .unwrap();
        w.plugin_call(
            "GET",
            &format!("/submissions/{draft}"),
            reader.token.expose(),
        )
        .send()
        .await
        .problem(404);

        // Granting the scope is all it takes: the same plugin can now report. A bad report is a
        // 422, and a draft is a 404 here too.
        {
            let mut conn = w.env.pool.acquire().await.unwrap();
            plugins::grant(
                &mut conn,
                "reader",
                &[Scope::SubmissionsRead, Scope::ReportsWrite],
            )
            .await
            .unwrap();
        }
        let path = format!("/submissions/{submission}/report");
        let put = |body: Value| {
            w.plugin_call("PUT", &path, reader.token.expose())
                .json(&body)
                .send()
        };
        assert_eq!(put(report.clone()).await.status, 204);
        put(json!({"summary": "", "data": {}})).await.problem(422);
        put(json!({"summary": "x", "data": [1]})).await.problem(422);
        w.plugin_call(
            "PUT",
            &format!("/submissions/{draft}/report"),
            reader.token.expose(),
        )
        .json(&report)
        .send()
        .await
        .problem(404);
        let reports = w.reports(submission).await;
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0]["plugin_id"], "reader");

        // A plugin that may only write cannot read what it reports on.
        let writer = w
            .install_without_a_process("writer", &[Scope::ReportsWrite])
            .await;
        w.plugin_call(
            "GET",
            &format!("/submissions/{submission}"),
            writer.token.expose(),
        )
        .send()
        .await
        .problem(403);

        // A wrong, malformed or missing token is a 401, whatever the call.
        common::Http::new(&w.env.base)
            .get(&format!("/plugin-api/submissions/{submission}"))
            .send()
            .await
            .problem(401);
        for token in ["nonsense".to_owned(), "vplg_".to_owned() + &"A".repeat(43)] {
            w.plugin_call("GET", &format!("/submissions/{submission}"), &token)
                .send()
                .await
                .problem(401);
        }
        // A session is not a plugin token, and a plugin token is not a session.
        w.plugin_call(
            "GET",
            &format!("/submissions/{submission}"),
            &w.teacher.token,
        )
        .send()
        .await
        .problem(401);
        w.env
            .call("GET", reader.token.expose(), "/me", None)
            .await
            .problem(401);

        // Who may read the reports: staff of the course, not its students, not outsiders.
        let reports_path = format!("/submissions/{submission}/plugin-reports");
        w.env
            .call("GET", &w.student.token, &reports_path, None)
            .await
            .problem(403);
        let outsider = w.env.user("out@x.edu", Role::Teacher).await;
        w.env
            .call("GET", &outsider.token, &reports_path, None)
            .await
            .problem(404);
        w.env
            .call(
                "GET",
                &w.teacher.token,
                &format!("/submissions/{draft}/plugin-reports"),
                None,
            )
            .await
            .problem(404);
    })
    .await;
}

#[tokio::test]
async fn disabling_a_plugin_stops_its_events_and_its_token() {
    db_test(async |pool| {
        let w = world(pool).await;
        let port = Plugin::free_port();
        let installed = {
            let bare = Plugin::start(port, &w.api, None).await;
            w.install(&bare.url(), &[Scope::SubmissionsRead, Scope::ReportsWrite])
                .await
        };
        let _plugin = Plugin::start(port, &w.api, Some(&installed)).await;

        // Working: the first work is delivered and reported.
        let first = w.submit("one two").await;
        assert_eq!(w.pump().await, 1);
        wait_for_report(&w, first).await;

        // Disabled: the next work owes the plugin nothing, and its token no longer works.
        {
            let mut conn = w.env.pool.acquire().await.unwrap();
            plugins::set_status(&mut conn, "hello", InstallStatus::Disabled)
                .await
                .unwrap();
        }
        let second = w.submit("one two three").await;
        assert_eq!(w.pump().await, 0, "nothing is sent to a disabled plugin");
        let owed: i64 = sqlx::query_scalar("SELECT count(*) FROM plugins.deliveries")
            .fetch_one(&w.env.pool)
            .await
            .unwrap();
        assert_eq!(owed, 1, "no delivery was even created for the second work");
        w.plugin_call(
            "GET",
            &format!("/submissions/{second}"),
            installed.token.expose(),
        )
        .send()
        .await
        .problem(401);
        assert!(w.reports(second).await.is_empty());
        // What it reported before stays.
        assert_eq!(w.reports(first).await.len(), 1);

        // Enabled again: the token works, and the next work is delivered.
        {
            let mut conn = w.env.pool.acquire().await.unwrap();
            plugins::set_status(&mut conn, "hello", InstallStatus::Enabled)
                .await
                .unwrap();
        }
        let third = w.submit("one two three four").await;
        assert_eq!(w.pump().await, 1);
        let report = wait_for_report(&w, third).await;
        assert_eq!(report["summary"], "4 words");
    })
    .await;
}

impl World {
    /// An install that has no process behind it: for the tests that call the plugin API with its
    /// token and send it nothing.
    async fn install_without_a_process(&self, id: &str, granted: &[Scope]) -> Installed {
        let manifest = parse_manifest(
            json!({"id": id, "version": "1", "api": "1", "scopes": ["submissions:read", "reports:write"]})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
        let key = self
            .state
            .plugins
            .key
            .as_deref()
            .expect("a plugin key is set");
        let mut conn = self.state.db.acquire().await.expect("a connection");
        plugins::install(&mut conn, key, "http://127.0.0.1:9", &manifest, granted)
            .await
            .expect("the install")
    }

    /// An install that offers a page in the tab of an assignment, with no process behind it.
    async fn install_with_a_page(&self, id: &str, base_url: &str) -> Installed {
        let manifest = parse_manifest(
            json!({"id": id, "version": "1", "api": "1", "ui": [
                {"slot": "assignment.tab", "title": format!("{id} tab"), "path": "/ui/page"}
            ]})
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        let key = self
            .state
            .plugins
            .key
            .as_deref()
            .expect("a plugin key is set");
        let mut conn = self.state.db.acquire().await.expect("a connection");
        plugins::install(&mut conn, key, base_url, &manifest, &[])
            .await
            .expect("the install")
    }

    /// Asks for the launch token of `plugin_id` in the tab of the assignment, as `token`'s owner.
    async fn launch(&self, token: &str, plugin_id: &str) -> common::Resp {
        self.env
            .call(
                "POST",
                token,
                &format!("/assignments/{}/plugin-launch", self.assignment),
                Some(json!({"plugin_id": plugin_id})),
            )
            .await
    }
}

/// The claims inside a launch token, without checking the signature (the plugin does that).
fn claims_of(token: &str) -> Value {
    use base64::Engine;
    let payload = token.split('.').next().expect("a payload");
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .expect("base64url");
    serde_json::from_slice(&bytes).expect("JSON claims")
}

#[tokio::test]
async fn the_slot_lists_the_pages_of_enabled_plugins_for_anyone_signed_in() {
    db_test(async |pool| {
        let w = world(pool).await;
        w.install_with_a_page("beta", "https://tools.example.edu/beta").await;
        w.install_with_a_page("alpha", "http://127.0.0.1:9000").await;
        w.install_without_a_process("quiet", &[]).await;
        let slots = |token: String, query: &'static str| {
            let env = &w.env;
            async move { env.call("GET", &token, &format!("/plugin-slots{query}"), None).await }
        };

        // A session is needed; a student has one; the answer is by plugin, with the address joined.
        common::Http::new(&w.env.base)
            .get("/plugin-slots?slot=assignment.tab")
            .send()
            .await
            .problem(401);
        let listed = slots(w.student.token.clone(), "?slot=assignment.tab").await;
        assert_eq!(listed.status, 200, "{}", String::from_utf8_lossy(&listed.body));
        assert_eq!(
            listed.json()["items"],
            json!([
                {"plugin_id": "alpha", "title": "alpha tab", "url": "http://127.0.0.1:9000/ui/page"},
                {"plugin_id": "beta", "title": "beta tab", "url": "https://tools.example.edu/beta/ui/page"},
            ])
        );

        // A disabled plugin vanishes, and comes back.
        let mut conn = w.env.pool.acquire().await.unwrap();
        plugins::set_status(&mut conn, "alpha", InstallStatus::Disabled).await.unwrap();
        let after = slots(w.teacher.token.clone(), "?slot=assignment.tab").await.json();
        assert_eq!(after["items"].as_array().unwrap().len(), 1);
        assert_eq!(after["items"][0]["plugin_id"], "beta");
        plugins::set_status(&mut conn, "alpha", InstallStatus::Enabled).await.unwrap();

        // A slot nobody can name is a 422 on `slot`, and so is no slot at all.
        let unknown = slots(w.student.token.clone(), "?slot=page.header").await.problem(422);
        assert_eq!(unknown.errors[0].field, "slot");
        slots(w.student.token.clone(), "").await.problem(422);
    })
    .await;
}

#[tokio::test]
async fn a_launch_token_says_who_looks_and_is_given_only_to_those_who_may_look() {
    db_test(async |pool| {
        let w = world(pool).await;
        w.install_with_a_page("hello", "http://127.0.0.1:9").await;
        w.install_without_a_process("blank", &[]).await;
        let admin = w.env.user("admin@x.edu", Role::Admin).await;
        let outsider = w.env.user("out@x.edu", Role::Teacher).await;

        // A student of the course is a `student`; its teacher and an admin are `staff`.
        let as_student = w.launch(&w.student.token, "hello").await;
        assert_eq!(
            as_student.status,
            200,
            "{}",
            String::from_utf8_lossy(&as_student.body)
        );
        assert_eq!(as_student.header("cache-control"), "no-store");
        let claims = claims_of(as_student.json()["token"].as_str().unwrap());
        assert_eq!(claims["role"], "student");
        assert_eq!(claims["plugin"], "hello");
        assert_eq!(claims["assignment"], json!(w.assignment));
        assert_eq!(claims["user"], json!(w.student.id));
        assert_eq!(
            claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(),
            60
        );
        let expires =
            chrono::DateTime::parse_from_rfc3339(as_student.json()["expires_at"].as_str().unwrap())
                .unwrap()
                .timestamp();
        assert_eq!(expires, claims["exp"].as_i64().unwrap());
        for token in [&w.teacher.token, &admin.token] {
            let staff = w.launch(token, "hello").await;
            assert_eq!(
                claims_of(staff.json()["token"].as_str().unwrap())["role"],
                "staff"
            );
        }

        // No session: 401. Somebody who may not see the assignment, a plugin that does not exist,
        // one with no tab, one that is disabled: all the same 404.
        common::Http::new(&w.env.base)
            .post(&format!("/assignments/{}/plugin-launch", w.assignment))
            .json(&json!({"plugin_id": "hello"}))
            .send()
            .await
            .problem(401);
        w.launch(&outsider.token, "hello").await.problem(404);
        w.launch(&w.student.token, "nobody").await.problem(404);
        w.launch(&w.student.token, "blank").await.problem(404);
        let nowhere = w
            .env
            .call(
                "POST",
                &w.student.token,
                &format!("/assignments/{}/plugin-launch", Uuid::now_v7()),
                Some(json!({"plugin_id": "hello"})),
            )
            .await;
        nowhere.problem(404);
        let mut conn = w.env.pool.acquire().await.unwrap();
        plugins::set_status(&mut conn, "hello", InstallStatus::Disabled)
            .await
            .unwrap();
        w.launch(&w.student.token, "hello").await.problem(404);

        // No plugin id is a 422.
        w.env
            .call(
                "POST",
                &w.student.token,
                &format!("/assignments/{}/plugin-launch", w.assignment),
                Some(json!({"plugin_id": ""})),
            )
            .await
            .problem(422);
    })
    .await;
}

#[tokio::test]
async fn the_example_page_trusts_only_a_token_the_core_signed() {
    db_test(async |pool| {
        let w = world(pool).await;
        let port = Plugin::free_port();
        let installed = {
            let bare = Plugin::start(port, &w.api, None).await;
            w.install(&bare.url(), &[Scope::SubmissionsRead, Scope::ReportsWrite])
                .await
        };
        let plugin = Plugin::start(port, &w.api, Some(&installed)).await;
        let data = async |token: &str| {
            reqwest::Client::new()
                .get(format!("{}/ui/data", plugin.url()))
                .header("authorization", format!("Launch {token}"))
                .send()
                .await
                .expect("the plugin answers")
        };
        let launch = async |who: &str| {
            let reply = w.launch(who, "hello").await;
            reply.json()["token"].as_str().expect("a token").to_owned()
        };

        // The page itself: a strict CSP of its own, and the answer to a preflight for the call.
        let page = reqwest::get(format!("{}/ui/words", plugin.url()))
            .await
            .unwrap();
        assert_eq!(page.status(), 200);
        let csp = page.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(csp.starts_with("default-src 'none'") && csp.contains("script-src 'sha256-"));
        assert!(!csp.contains("unsafe-inline") && !csp.contains("unsafe-eval"));
        let preflight = reqwest::Client::new()
            .request(
                reqwest::Method::OPTIONS,
                format!("{}/ui/data", plugin.url()),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(preflight.status(), 204);
        assert_eq!(preflight.headers()["access-control-allow-origin"], "*");

        // A teacher's token gets the totals; they are empty until a work is reported.
        let teacher = launch(&w.teacher.token).await;
        let empty = data(&teacher).await;
        assert_eq!(empty.status(), 200);
        assert_eq!(empty.headers()["access-control-allow-origin"], "*");
        let body: Value = serde_json::from_slice(&empty.bytes().await.unwrap()).unwrap();
        assert_eq!(body, json!({"role": "staff", "submissions": 0, "words": 0}));
        let submission = w.submit("one two three").await;
        assert_eq!(w.pump().await, 1);
        wait_for_report(&w, submission).await;
        let counted =
            serde_json::from_slice::<Value>(&data(&teacher).await.bytes().await.unwrap()).unwrap();
        assert_eq!(
            counted,
            json!({"role": "staff", "submissions": 1, "words": 3})
        );

        // A student's token is a student: no numbers.
        let student = launch(&w.student.token).await;
        let for_student =
            serde_json::from_slice::<Value>(&data(&student).await.bytes().await.unwrap()).unwrap();
        assert_eq!(for_student, json!({"role": "student"}));

        // A token the core did not sign, one with a changed payload, one cut short, and none.
        let (payload, signature) = teacher.split_once('.').unwrap();
        let flipped = format!(
            "{}{}",
            &signature[..signature.len() - 1],
            if signature.ends_with('0') { '1' } else { '0' }
        );
        let as_student_payload = student.split_once('.').unwrap().0;
        for bad in [
            format!("{payload}.{flipped}"),
            format!("{as_student_payload}.{signature}"),
            payload.to_owned(),
            "garbage".to_owned(),
            String::new(),
        ] {
            assert_eq!(data(&bad).await.status(), 401, "{bad}");
        }
        // The signature of an event over the same bytes is not a token (the prefix differs).
        let as_event = plugins::sign(&installed.secret, 1_700_000_000, payload.as_bytes()).unwrap();
        let as_event = as_event.trim_start_matches("sha256=");
        assert_eq!(data(&format!("{payload}.{as_event}")).await.status(), 401);
    })
    .await;
}

async fn stored_reports(w: &World) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM plugins.reports")
        .fetch_one(&w.env.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_report_is_replaced_by_the_next_and_goes_with_its_attempt_and_with_its_plugin() {
    db_test(async |pool| {
        let w = world(pool).await;
        let plugin = w
            .install_without_a_process("writer", &[Scope::ReportsWrite])
            .await;
        let first = w.submit("a b c").await;
        let second = w.submit("d e").await;
        let report = |submission: Uuid, summary: &'static str| {
            let call = w.plugin_call(
                "PUT",
                &format!("/submissions/{submission}/report"),
                plugin.token.expose(),
            );
            async move {
                call.json(&json!({"summary": summary, "data": {}}))
                    .send()
                    .await
            }
        };

        // A second report on the same attempt replaces the first; another attempt has its own.
        assert_eq!(report(first, "3 words").await.status, 204);
        assert_eq!(report(first, "three words").await.status, 204);
        assert_eq!(report(second, "2 words").await.status, 204);
        let on_first = w.reports(first).await;
        assert_eq!(
            (on_first.len(), on_first[0]["summary"].as_str()),
            (1, Some("three words"))
        );
        assert_eq!(stored_reports(&w).await, 2);

        // The attempt goes (nothing does this today; the erasure of a student's data will): its report
        // goes in the same statement, the other attempt's stays, and a new one cannot be written.
        sqlx::query("DELETE FROM assessment.submissions WHERE id = $1")
            .bind(first)
            .execute(&w.env.pool)
            .await
            .unwrap();
        assert_eq!(stored_reports(&w).await, 1);
        assert!(w.reports(second).await.len() == 1);
        report(first, "again").await.problem(404);
        assert_eq!(stored_reports(&w).await, 1);

        // The plugin goes: so do its reports.
        let mut conn = w.env.pool.acquire().await.unwrap();
        plugins::remove(&mut conn, "writer").await.unwrap();
        assert_eq!(stored_reports(&w).await, 0);
    })
    .await;
}
