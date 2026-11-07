//! ADR-0078: people reach routes of a plugin's own server through the core. The core checks the
//! session and the roles the plugin declared, signs who the caller is, passes nothing of the session
//! on, and passes the answer on only if it cannot run in a browser. The plugin here is the real example
//! (`examples/hello-plugin/plugin.py`, which needs `python3`) and a recording server for what the
//! core sends.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use ::common::Role;
use access::CourseScope;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::Response;
use common::{Account, Env, db_test, spawn_app, test_settings, wait_until};
use plugins::{Installed, ProxyClaims, proxy_signature};
use serde_json::{Value, json};
use server::app::AppState;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tokio::net::TcpStream;
use uuid::Uuid;
use vumlo_config::Secret;

const KEY: &str = "0123456789abcdef0123456789abcdef";

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

    async fn start(port: u16, credentials: Option<&Installed>) -> Self {
        let mut command = Command::new("python3");
        command
            .arg(plugin_script())
            .env("PORT", port.to_string())
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

/// One call a recording server was sent: method, request-target, headers, body.
type Seen = (Method, String, HeaderMap, Vec<u8>);

/// What a recording server answers: status, content type, body, other headers.
type Reply = (
    u16,
    Option<&'static str>,
    Vec<u8>,
    Vec<(&'static str, &'static str)>,
);

/// What a recording server was sent, and what it answers.
#[derive(Clone)]
struct Recorder {
    seen: Arc<Mutex<Vec<Seen>>>,
    reply: Arc<Mutex<Reply>>,
}

async fn record(State(recorder): State<Recorder>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let bytes: Bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    let target = parts
        .uri
        .path_and_query()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_default();
    recorder
        .seen
        .lock()
        .unwrap()
        .push((parts.method, target, parts.headers, bytes.to_vec()));
    let (status, content_type, body, extra) = recorder.reply.lock().unwrap().clone();
    let mut response = Response::new(axum::body::Body::from(body));
    *response.status_mut() = StatusCode::from_u16(status).unwrap();
    if let Some(content_type) = content_type {
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, content_type.parse().unwrap());
    }
    for (name, value) in extra {
        response.headers_mut().insert(name, value.parse().unwrap());
    }
    response
}

/// A server that records every call and answers `reply`. Its manifest is `manifest`.
async fn recording_server(manifest: Value) -> (String, Recorder) {
    let recorder = Recorder {
        seen: Arc::default(),
        reply: Arc::new(Mutex::new((
            200,
            Some("application/json"),
            br#"{"ok":true}"#.to_vec(),
            Vec::new(),
        ))),
    };
    let app = Router::new()
        .route(
            "/manifest",
            axum::routing::get(move || {
                let manifest = manifest.clone();
                async move { axum::Json(manifest) }
            }),
        )
        .fallback(record)
        .with_state(recorder.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });
    (url, recorder)
}

struct World {
    env: Env,
    state: AppState,
    teacher: Account,
    assistant: Account,
    student: Account,
    admin: Account,
    stranger: Account,
    course: Uuid,
    other_course: Uuid,
}

async fn world(pool: PgPool) -> World {
    let mut settings = test_settings();
    settings.plugins.secret_key = Secret::new(KEY);
    let (base, state) = spawn_app(pool.clone(), settings).await;
    let env = Env {
        http: common::Http::new(&base),
        pool,
        base,
    };
    let teacher = env.user("teacher@x.edu", Role::Teacher).await;
    let assistant = env.user("assistant@x.edu", Role::Teacher).await;
    let student = env.user("student@x.edu", Role::Student).await;
    let admin = env.user("admin@x.edu", Role::Admin).await;
    let stranger = env.user("stranger@x.edu", Role::Teacher).await;
    let (course, _) = env.course(&teacher, &[&student]).await;
    env.ok(
        "PUT",
        &teacher.token,
        &format!("/courses/{course}/members/{}", assistant.id),
        Some(json!({"role": "assistant"})),
        200,
    )
    .await;
    let (other_course, _) = env.course(&teacher, &[&student]).await;
    World {
        env,
        state,
        teacher,
        assistant,
        student,
        admin,
        stranger,
        course,
        other_course,
    }
}

impl World {
    async fn install(&self, url: &str, courses: CourseScope) -> Installed {
        let manifest = plugins::fetch_manifest(url).await.expect("the manifest");
        let key = self.state.plugins.key.as_deref().expect("a plugin key");
        let mut conn = self.state.db.acquire().await.unwrap();
        plugins::install(&mut conn, key, url, &manifest, &[], &courses)
            .await
            .expect("the install")
    }

    /// The example plugin, installed and running with its credentials.
    async fn example(&self, courses: CourseScope) -> (Installed, Plugin) {
        let port = Plugin::free_port();
        let installed = {
            let bare = Plugin::start(port, None).await;
            self.install(&bare.url(), courses).await
        };
        let running = Plugin::start(port, Some(&installed)).await;
        (installed, running)
    }

    async fn call(&self, who: Option<&Account>, method: &str, path: &str) -> common::Resp {
        let http = common::Http::new(&self.env.base);
        let url = format!("/plugins/{path}");
        let request = match method {
            "POST" => http.post(&url),
            "PUT" => http.put(&url),
            "DELETE" => http.delete(&url),
            _ => http.get(&url),
        };
        match who {
            Some(who) => request.bearer(&who.token).send().await,
            None => request.send().await,
        }
    }

    async fn summary(&self, who: Option<&Account>, course: Uuid) -> common::Resp {
        self.call(who, "GET", &format!("hello/api/summary/{course}"))
            .await
    }
}

fn echo_manifest() -> Value {
    json!({"id": "rec", "version": "1", "api": "1", "routes": [
        {"method": "GET", "path": "/r/{course_id}", "roles": ["admin", "teacher", "student"],
         "course_roles": ["teacher", "assistant", "student"]},
        {"method": "POST", "path": "/r/note", "roles": ["admin", "teacher", "student"]},
        {"method": "GET", "path": "/open", "roles": ["admin", "teacher", "student"]},
    ]})
}

// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_route_is_open_to_the_roles_it_lists_and_to_the_course_roles_in_its_course() {
    db_test(async |pool| {
        let w = world(pool).await;
        let (_, _plugin) = w.example(CourseScope::All).await;

        // The teacher of the course, an assistant who is a teacher at the institution, and an
        // administrator: the plugin answers, and it only does when the core's signature is good.
        for (who, role, course_role) in [
            (&w.teacher, "teacher", json!("teacher")),
            (&w.assistant, "teacher", json!("assistant")),
            (&w.admin, "admin", Value::Null),
        ] {
            let response = w.summary(Some(who), w.course).await;
            assert_eq!(
                response.status,
                200,
                "{}",
                String::from_utf8_lossy(&response.body)
            );
            let body = response.json();
            assert_eq!(body["user"], json!(who.id));
            assert_eq!(body["role"], role);
            assert_eq!(body["course"], json!(w.course));
            assert_eq!(body["course_role"], course_role);
            // The answer of a plugin is never cached or sniffed, and cannot run script here.
            assert_eq!(response.header("cache-control"), "no-store");
            assert_eq!(response.header("x-content-type-options"), "nosniff");
            assert!(
                response
                    .header("content-security-policy")
                    .contains("sandbox")
            );
        }
        // A student of the course: the route is not for students. A teacher who is no member does
        // not know the course is there. No session: 401.
        w.summary(Some(&w.student), w.course).await.problem(403);
        w.summary(Some(&w.stranger), w.course).await.problem(404);
        w.summary(None, w.course).await.problem(401);
        // A course that is not there, or not a course.
        w.summary(Some(&w.teacher), Uuid::now_v7())
            .await
            .problem(404);
        w.call(Some(&w.teacher), "GET", "hello/api/summary/not-a-course")
            .await
            .problem(404);
        // Not declared: another path, another method, another plugin, the manifest itself.
        w.call(
            Some(&w.teacher),
            "POST",
            &format!("hello/api/summary/{}", w.course),
        )
        .await
        .problem(404);
        w.call(Some(&w.teacher), "GET", "hello/api/other")
            .await
            .problem(404);
        w.call(Some(&w.teacher), "GET", "hello/manifest")
            .await
            .problem(404);
        w.call(Some(&w.teacher), "GET", "hello/events")
            .await
            .problem(404);
        w.call(Some(&w.teacher), "GET", "nobody/api/echo")
            .await
            .problem(404);
        // Nothing that could climb out of the declared route reaches the plugin.
        for path in [
            "hello/api/summary/..",
            "hello/api/summary/%2e%2e",
            "hello/api/summary/a%2fb",
            "hello/api/../manifest",
            "hello/api/%2e%2e/manifest",
        ] {
            w.call(Some(&w.teacher), "GET", path).await.problem(404);
        }
        // The route that lists every role: a student may use it.
        let echo = w.call(Some(&w.student), "POST", "hello/api/echo").await;
        assert_eq!(echo.status, 200, "{}", String::from_utf8_lossy(&echo.body));
        assert_eq!(echo.json()["role"], "student");
        assert_eq!(echo.json()["course"], Value::Null);
    })
    .await;
}

#[tokio::test]
async fn a_course_the_plugin_may_not_reach_and_a_disabled_plugin_are_refused() {
    db_test(async |pool| {
        let w = world(pool).await;
        let (_, _plugin) = w.example(CourseScope::Only(vec![w.course])).await;
        assert_eq!(w.summary(Some(&w.teacher), w.course).await.status, 200);
        // The teacher sees the other course; the plugin is not granted it.
        w.summary(Some(&w.teacher), w.other_course)
            .await
            .problem(403);
        // Nobody who cannot see the course learns the grant either.
        w.summary(Some(&w.stranger), w.other_course)
            .await
            .problem(404);

        let mut conn = w.state.db.acquire().await.unwrap();
        plugins::set_status(&mut conn, "hello", plugins::InstallStatus::Disabled)
            .await
            .unwrap();
        drop(conn);
        w.summary(Some(&w.teacher), w.course).await.problem(404);
        let mut conn = w.state.db.acquire().await.unwrap();
        plugins::set_status(&mut conn, "hello", plugins::InstallStatus::Enabled)
            .await
            .unwrap();
        drop(conn);
        assert_eq!(w.summary(Some(&w.teacher), w.course).await.status, 200);
    })
    .await;
}

#[tokio::test]
async fn the_plugin_sees_the_body_and_the_query_but_never_a_cookie_or_a_session_or_a_forged_header()
{
    db_test(async |pool| {
        let w = world(pool).await;
        let (_, _plugin) = w.example(CourseScope::All).await;
        let body = "{\"note\":\"hello é\"}".as_bytes().to_vec();
        let response = common::Http::new(&w.env.base)
            .post("/plugins/hello/api/echo?x=1&y=two")
            .bearer(&w.student.token)
            .header("cookie", "vumlo_session=SECRET-SESSION; other=1")
            .header("x-vumlo-role", "admin")
            .header("x-vumlo-user", &w.teacher.id.to_string())
            .header("x-vumlo-course", &w.course.to_string())
            .header("content-type", "application/json")
            .header("x-something-else", "x")
            .body(body.clone())
            .send()
            .await;
        assert_eq!(
            response.status,
            200,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        let seen = response.json();
        // Who it is: the student, not what the caller wrote in the headers.
        assert_eq!(seen["user"], json!(w.student.id));
        assert_eq!(seen["role"], "student");
        assert_eq!(seen["course"], Value::Null);
        assert_eq!(seen["body_bytes"], body.len());
        assert_eq!(
            seen["body_sha256"],
            Sha256::digest(&body)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let headers: Vec<String> = seen["headers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h.as_str().unwrap().to_owned())
            .collect();
        for never in ["cookie", "authorization", "x-something-else"] {
            assert!(
                !headers.contains(&never.to_owned()),
                "{never} reached the plugin: {headers:?}"
            );
        }
        for always in [
            "x-vumlo-user",
            "x-vumlo-role",
            "x-vumlo-timestamp",
            "x-vumlo-nonce",
            "x-vumlo-signature",
            "content-type",
        ] {
            assert!(
                headers.contains(&always.to_owned()),
                "{always}: {headers:?}"
            );
        }
        assert!(
            !headers.contains(&"x-vumlo-course".to_owned()),
            "a course nobody named"
        );
    })
    .await;
}

#[tokio::test]
async fn every_call_is_signed_over_what_it_is_and_carries_a_nonce_nobody_has_used() {
    db_test(async |pool| {
        let w = world(pool).await;
        let (url, recorder) = recording_server(echo_manifest()).await;
        let installed = w.install(&url, CourseScope::All).await;
        let before = chrono::Utc::now().timestamp();

        for _ in 0..2 {
            let response = w
                .call(Some(&w.assistant), "GET", &format!("rec/r/{}", w.course))
                .await;
            assert_eq!(
                response.status,
                200,
                "{}",
                String::from_utf8_lossy(&response.body)
            );
        }
        let posted = common::Http::new(&w.env.base)
            .post("/plugins/rec/r/note?z=1")
            .bearer(&w.student.token)
            .body(b"some bytes".to_vec())
            .send()
            .await;
        assert_eq!(posted.status, 200);

        let seen = recorder.seen.lock().unwrap().clone();
        // The first fetch of the manifest, then the three calls.
        let calls: Vec<_> = seen.iter().filter(|c| c.1 != "/manifest").collect();
        assert_eq!(calls.len(), 3);
        let mut nonces = Vec::new();
        for (method, target, headers, body) in calls {
            let get = |name: &str| headers.get(name).map(|v| v.to_str().unwrap().to_owned());
            let nonce: Uuid = get("x-vumlo-nonce").unwrap().parse().unwrap();
            nonces.push(nonce);
            let timestamp: i64 = get("x-vumlo-timestamp").unwrap().parse().unwrap();
            assert!(timestamp >= before && timestamp <= chrono::Utc::now().timestamp());
            let user: Uuid = get("x-vumlo-user").unwrap().parse().unwrap();
            let course = get("x-vumlo-course").map(|c| c.parse::<Uuid>().unwrap());
            let course_role = get("x-vumlo-course-role");
            let role = get("x-vumlo-role").unwrap();
            // The plugin can check the signature with the secret it was given and nothing else.
            let expected = proxy_signature(
                &installed.secret,
                &ProxyClaims {
                    method: method.as_str(),
                    target,
                    user,
                    role: &role,
                    course,
                    course_role: course_role.as_deref(),
                    nonce,
                    timestamp,
                },
                body,
            )
            .unwrap();
            assert_eq!(
                get("x-vumlo-signature").unwrap(),
                expected,
                "{method} {target}"
            );
            // And it names the people it should.
            if method == Method::GET {
                assert_eq!(user, w.assistant.id);
                assert_eq!(role, "teacher");
                assert_eq!(course, Some(w.course));
                assert_eq!(course_role.as_deref(), Some("assistant"));
            } else {
                assert_eq!((user, role.as_str()), (w.student.id, "student"));
                assert_eq!(target, "/r/note?z=1");
                assert_eq!(body, b"some bytes");
                assert_eq!(course, None);
            }
            assert!(get("cookie").is_none() && get("authorization").is_none());
        }
        nonces.sort();
        nonces.dedup();
        assert_eq!(nonces.len(), 3, "a nonce was used twice");
    })
    .await;
}

#[tokio::test]
async fn the_example_plugin_refuses_a_call_that_is_old_forged_or_seen_before() {
    db_test(async |pool| {
        let w = world(pool).await;
        let (installed, plugin) = w.example(CourseScope::All).await;
        let user = w.teacher.id;
        let now = chrono::Utc::now().timestamp();
        let send =
            async |timestamp: i64, nonce: Uuid, role: &str, signed_role: &str, body: &[u8]| {
                let target = "/api/echo";
                let signature = proxy_signature(
                    &installed.secret,
                    &ProxyClaims {
                        method: "POST",
                        target,
                        user,
                        role: signed_role,
                        course: None,
                        course_role: None,
                        nonce,
                        timestamp,
                    },
                    body,
                )
                .unwrap();
                reqwest::Client::new()
                    .post(format!("{}{target}", plugin.url()))
                    .header("x-vumlo-user", user.to_string())
                    .header("x-vumlo-role", role)
                    .header("x-vumlo-timestamp", timestamp.to_string())
                    .header("x-vumlo-nonce", nonce.to_string())
                    .header("x-vumlo-signature", signature)
                    .body(body.to_vec())
                    .send()
                    .await
                    .unwrap()
                    .status()
                    .as_u16()
            };
        let nonce = Uuid::now_v7();
        // A fresh call, signed with the secret of the install: taken, once.
        assert_eq!(send(now, nonce, "teacher", "teacher", b"x").await, 200);
        // The same call again, as a thief who captured it would send it: refused.
        assert_eq!(send(now, nonce, "teacher", "teacher", b"x").await, 401);
        // Old, or from the future: refused although it is signed and new.
        for skewed in [now - 600, now + 600] {
            assert_eq!(
                send(skewed, Uuid::now_v7(), "teacher", "teacher", b"x").await,
                401
            );
        }
        // The role changed on the way: the signature no longer fits.
        assert_eq!(
            send(now, Uuid::now_v7(), "admin", "teacher", b"x").await,
            401
        );
        // Signed with another secret, or not signed.
        let forged = reqwest::Client::new()
            .post(format!("{}/api/echo", plugin.url()))
            .header("x-vumlo-user", user.to_string())
            .header("x-vumlo-role", "admin")
            .header("x-vumlo-timestamp", now.to_string())
            .header("x-vumlo-nonce", Uuid::now_v7().to_string())
            .header("x-vumlo-signature", "sha256=00")
            .send()
            .await
            .unwrap();
        assert_eq!(forged.status().as_u16(), 401);
        let bare = reqwest::Client::new()
            .post(format!("{}/api/echo", plugin.url()))
            .send()
            .await
            .unwrap();
        assert_eq!(bare.status().as_u16(), 401);
    })
    .await;
}

#[tokio::test]
async fn what_the_plugin_answers_is_passed_on_only_if_it_cannot_run_in_a_browser() {
    db_test(async |pool| {
        let w = world(pool).await;
        let (url, recorder) = recording_server(echo_manifest()).await;
        w.install(&url, CourseScope::All).await;
        let set = |status: u16,
                   content_type: Option<&'static str>,
                   body: &[u8],
                   extra: &[(&'static str, &'static str)]| {
            *recorder.reply.lock().unwrap() = (status, content_type, body.to_vec(), extra.to_vec());
        };
        let get = async || w.call(Some(&w.student), "GET", "rec/open").await;

        // JSON, text, CSV, a calendar, a PDF and a spreadsheet are passed on, with their type.
        for content_type in [
            "application/json",
            "application/json; charset=utf-8",
            "text/plain",
            "text/csv",
            "text/calendar",
            "application/pdf",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ] {
            set(200, Some(content_type), b"data", &[]);
            let response = get().await;
            assert_eq!(response.status, 200, "{content_type}");
            assert_eq!(response.header("content-type"), content_type);
            assert_eq!(response.body, b"data");
            assert_eq!(response.header("x-content-type-options"), "nosniff");
            assert_eq!(response.header("cache-control"), "no-store");
        }
        // A page, a script, an image that is also a script, an unknown type, no type: not passed on.
        for content_type in [
            Some("text/html"),
            Some("text/html; charset=utf-8"),
            Some("TEXT/HTML"),
            Some("application/xhtml+xml"),
            Some("image/svg+xml"),
            Some("application/javascript"),
            Some("text/javascript"),
            Some("application/octet-stream"),
            None,
        ] {
            set(200, content_type, b"<script>alert(1)</script>", &[]);
            let response = get().await;
            assert_eq!(response.status, 502, "{content_type:?}");
            assert!(!String::from_utf8_lossy(&response.body).contains("script"));
        }
        // The plugin's own 4xx is its to say, in its own words.
        set(422, Some("application/json"), br#"{"field":"name"}"#, &[]);
        let refused = get().await;
        assert_eq!(refused.status, 422);
        assert_eq!(refused.body, br#"{"field":"name"}"#);
        set(404, Some("application/json"), b"{}", &[]);
        assert_eq!(get().await.status, 404);
        // An answer with no body needs no type.
        set(204, None, b"", &[]);
        assert_eq!(get().await.status, 204);
        // Its failures are not the person's: a 5xx, a redirect, a 1xx-style odd status are a 502.
        for status in [500, 503, 301, 302, 307] {
            set(
                status,
                Some("application/json"),
                b"{}",
                &[("location", "http://evil.test/")],
            );
            let response = get().await;
            assert_eq!(response.status, 502, "{status}");
            assert!(response.headers.get("location").is_none());
        }
        // A download keeps its name; any other disposition is dropped.
        set(
            200,
            Some("application/pdf"),
            b"%PDF",
            &[("content-disposition", "attachment; filename=\"a.pdf\"")],
        );
        assert_eq!(
            get().await.header("content-disposition"),
            "attachment; filename=\"a.pdf\""
        );
        set(
            200,
            Some("text/plain"),
            b"x",
            &[("content-disposition", "inline")],
        );
        assert!(get().await.headers.get("content-disposition").is_none());
        // What the plugin set for itself is not passed on: cookies, CSP, caching.
        set(
            200,
            Some("text/plain"),
            b"x",
            &[
                ("set-cookie", "a=b"),
                ("content-security-policy", "default-src *"),
                ("cache-control", "public"),
            ],
        );
        let response = get().await;
        assert!(response.headers.get("set-cookie").is_none());
        assert_eq!(response.header("cache-control"), "no-store");
        assert!(
            response
                .header("content-security-policy")
                .contains("sandbox")
        );
    })
    .await;
}

#[tokio::test]
async fn a_plugin_that_is_down_or_a_body_that_is_too_large_is_told_plainly() {
    db_test(async |pool| {
        let w = world(pool).await;
        let (url, recorder) = recording_server(echo_manifest()).await;
        w.install(&url, CourseScope::All).await;
        // Too large: 413, and the plugin was never called.
        let calls = || {
            recorder
                .seen
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c.1 != "/manifest")
                .count()
        };
        let big = common::Http::new(&w.env.base)
            .post("/plugins/rec/r/note")
            .bearer(&w.student.token)
            .body(vec![b'x'; 10 * 1024 * 1024 + 1])
            .send()
            .await;
        assert_eq!(big.status, 413);
        assert_eq!(calls(), 0);
        // Just inside the limit it goes through.
        let ok = common::Http::new(&w.env.base)
            .post("/plugins/rec/r/note")
            .bearer(&w.student.token)
            .body(vec![b'x'; 10 * 1024 * 1024])
            .send()
            .await;
        assert_eq!(ok.status, 200);
        assert_eq!(calls(), 1);
        // A plugin nobody listens for any more: 502, with a problem document.
        sqlx::query("UPDATE plugins.plugin_installs SET base_url = 'http://127.0.0.1:9'")
            .execute(&w.env.pool)
            .await
            .unwrap();
        let down = w.call(Some(&w.student), "GET", "rec/open").await;
        assert_eq!(down.status, 502);
        down.problem(502);
    })
    .await;
}

#[tokio::test]
async fn a_call_with_a_session_cookie_is_checked_for_where_it_came_from_like_any_write() {
    db_test(async |pool| {
        let w = world(pool).await;
        let (url, recorder) = recording_server(echo_manifest()).await;
        w.install(&url, CourseScope::All).await;
        let cookie = common::login_web(
            &common::Http::new(&w.env.base),
            "student@x.edu",
            common::PASSWORD,
        )
        .await;
        let calls = || {
            recorder
                .seen
                .lock()
                .unwrap()
                .iter()
                .filter(|c| c.1 != "/manifest")
                .count()
        };
        let post = |origin: Option<&str>| {
            let request = common::Http::new(&w.env.base)
                .post("/plugins/rec/r/note")
                .cookie(&cookie)
                .body(b"x".to_vec());
            match origin {
                Some(origin) => request.origin(origin).send(),
                None => request.send(),
            }
        };
        // A page of another origin, or none: refused before the plugin hears of it.
        assert_eq!(post(Some("https://evil.example")).await.status, 403);
        assert_eq!(post(None).await.status, 403);
        assert_eq!(calls(), 0, "the plugin was called for a forged request");
        // The web app's own origin is let through, and the cookie goes no further than the core.
        assert_eq!(post(Some(common::ORIGIN)).await.status, 200);
        assert_eq!(calls(), 1);
        let seen = recorder.seen.lock().unwrap().clone();
        let headers = &seen.iter().find(|c| c.1 != "/manifest").unwrap().2;
        assert!(headers.get("cookie").is_none() && headers.get("authorization").is_none());
    })
    .await;
}
