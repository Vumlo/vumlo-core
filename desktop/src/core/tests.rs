use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::*;

// --- a server that can be switched off -----------------------------------------------------------

#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

type Answer = (u16, Vec<(&'static str, String)>, String);

struct Mock {
    port: u16,
    seen: Arc<Mutex<Vec<Seen>>>,
    answers: Arc<Mutex<VecDeque<Answer>>>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Mock {
    async fn start() -> Self {
        let mock = Self {
            port: 0,
            seen: Arc::default(),
            answers: Arc::default(),
            task: Mutex::new(None),
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut mock = Self { port, ..mock };
        mock.serve(listener);
        mock.port = port;
        mock
    }

    fn serve(&self, listener: TcpListener) {
        let (seen, answers) = (self.seen.clone(), self.answers.clone());
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let (seen, answers) = (seen.clone(), answers.clone());
                tokio::spawn(async move {
                    let mut buffer = Vec::new();
                    let mut chunk = [0_u8; 4096];
                    let head_end = loop {
                        let n = socket.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buffer.extend_from_slice(&chunk[..n]);
                        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                            break at + 4;
                        }
                    };
                    let head = String::from_utf8_lossy(&buffer[..head_end]).to_string();
                    let mut lines = head.lines();
                    let mut first = lines.next().unwrap_or_default().split(' ');
                    let (method, path) = (
                        first.next().unwrap_or_default().to_owned(),
                        first.next().unwrap_or_default().to_owned(),
                    );
                    let headers: Vec<(String, String)> = lines
                        .filter_map(|l| l.split_once(':'))
                        .map(|(n, v)| (n.trim().to_owned(), v.trim().to_owned()))
                        .collect();
                    let length: usize = headers
                        .iter()
                        .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, v)| v.parse().ok())
                        .unwrap_or(0);
                    while buffer.len() < head_end + length {
                        let n = socket.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buffer.extend_from_slice(&chunk[..n]);
                    }
                    let body = String::from_utf8_lossy(&buffer[head_end..]).to_string();
                    seen.lock().unwrap().push(Seen {
                        method,
                        path,
                        headers,
                        body,
                    });
                    let (status, extra, text) = answers.lock().unwrap().pop_front().unwrap_or((
                        200,
                        vec![],
                        "{}".to_owned(),
                    ));
                    let mut response = format!(
                        "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\ncontent-type: application/json\r\n",
                        text.len()
                    );
                    for (name, value) in extra {
                        response.push_str(&format!("{name}: {value}\r\n"));
                    }
                    response.push_str("\r\n");
                    response.push_str(&text);
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        *self.task.lock().unwrap() = Some(task);
    }

    /// Connections are refused until `up`.
    fn down(&self) {
        if let Some(task) = self.task.lock().unwrap().take() {
            task.abort();
        }
    }

    async fn up(&self) {
        let listener = TcpListener::bind(("127.0.0.1", self.port)).await.unwrap();
        self.serve(listener);
    }

    fn answer(&self, status: u16, body: Value) {
        self.answers
            .lock()
            .unwrap()
            .push_back((status, vec![], body.to_string()));
    }

    fn answer_with(&self, status: u16, headers: Vec<(&'static str, String)>, body: Value) {
        self.answers
            .lock()
            .unwrap()
            .push_back((status, headers, body.to_string()));
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

struct Rig {
    core: Arc<Core>,
    mock: Mock,
    events: Arc<Mutex<Vec<SyncState>>>,
    token: Arc<MemoryToken>,
}

async fn rig() -> Rig {
    let mock = Mock::start().await;
    let events: Arc<Mutex<Vec<SyncState>>> = Arc::default();
    let sink = events.clone();
    let token = Arc::new(MemoryToken::default());
    let core = Arc::new(Core::new(
        Store::open_in_memory().unwrap(),
        format!("http://127.0.0.1:{}", mock.port),
        token.clone(),
        Box::new(move |state| sink.lock().unwrap().push(state.clone())),
    ));
    Rig {
        core,
        mock,
        events,
        token,
    }
}

fn get(path: &str) -> ApiRequest {
    ApiRequest {
        method: "GET".into(),
        path: path.into(),
        ..Default::default()
    }
}

fn queued(method: &str, path: &str, body: Option<Value>, key: &str) -> ApiRequest {
    ApiRequest {
        method: method.into(),
        path: path.into(),
        body,
        queue: true,
        idem_key: Some(key.into()),
    }
}

async fn sign_in(rig: &Rig, user: &str) {
    rig.mock.answer(
        200,
        json!({"token": "T0KEN", "expires_at": "x", "user": {"id": user, "name": "Maya"}}),
    );
    let reply = rig
        .core
        .request(ApiRequest {
            method: "POST".into(),
            path: "/auth/login".into(),
            body: Some(json!({"email": "a", "password": "b", "client": "app"})),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(reply.status, 200);
}

const DRAFT: &str = "/assignments/a1/submission/draft";
const SUBMIT: &str = "/assignments/a1/submission/submit";

// --- pure helpers ---------------------------------------------------------------------------------

#[test]
fn only_paths_inside_the_api_are_accepted() {
    assert!(valid_path("/me"));
    assert!(valid_path("/courses/abc/content?x=1"));
    for bad in [
        "me",
        "//evil.example/x",
        "/../x",
        "/a/../b",
        "/a\\b",
        "/x://y",
        "",
    ] {
        assert!(!valid_path(bad), "{bad}");
    }
}

#[test]
fn the_back_off_doubles_to_five_minutes() {
    let waits: Vec<_> = (1..=9).map(backoff_secs).collect();
    assert_eq!(waits, [5, 10, 20, 40, 80, 160, 300, 300, 300]);
    assert_eq!(backoff_secs(0), 5);
}

#[test]
fn a_time_is_written_as_the_server_writes_it() {
    assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
    assert_eq!(rfc3339(1_700_000_000_000), "2023-11-14T22:13:20Z");
    assert_eq!(
        rfc3339(951_782_400_000),
        "2000-02-29T00:00:00Z",
        "a leap day"
    );
    assert_eq!(rfc3339(1_709_164_799_999), "2024-02-28T23:59:59Z");
}

// --- reading --------------------------------------------------------------------------------------

#[tokio::test]
async fn an_answer_is_kept_confirmed_by_a_304_and_served_when_the_server_is_gone() {
    let rig = rig().await;
    rig.mock.answer_with(
        200,
        vec![("etag", "W/\"1\"".into())],
        json!({"title": "Algebra"}),
    );
    let first = rig.core.request(get("/courses/c1")).await.unwrap();
    assert_eq!(
        (first.status, first.from_cache, first.body.clone()),
        (200, false, Some(json!({"title": "Algebra"})))
    );

    rig.mock.answer(304, json!(null));
    let second = rig.core.request(get("/courses/c1")).await.unwrap();
    assert_eq!(
        (second.status, second.from_cache, second.body),
        (200, false, Some(json!({"title": "Algebra"})))
    );
    assert_eq!(rig.mock.seen()[1].header("if-none-match"), Some("W/\"1\""));

    rig.mock.down();
    let third = rig.core.request(get("/courses/c1")).await.unwrap();
    assert!(third.from_cache && third.cached_at.is_some());
    assert_eq!(third.body, Some(json!({"title": "Algebra"})));
    assert!(!rig.core.state().online);

    // Nothing was ever read of this one: the failure is the caller's to show.
    assert!(rig.core.request(get("/courses/c2")).await.is_err());
}

#[tokio::test]
async fn a_server_error_serves_the_copy_and_a_refusal_drops_it() {
    let rig = rig().await;
    rig.mock.answer(200, json!({"n": 1}));
    rig.core.request(get("/courses/c1")).await.unwrap();
    rig.mock.answer(503, json!({"title": "busy"}));
    let busy = rig.core.request(get("/courses/c1")).await.unwrap();
    assert_eq!((busy.status, busy.from_cache), (200, true));

    // The course was unpublished: the server wins, the copy is gone.
    rig.mock.answer(404, json!({"title": "Not Found"}));
    assert_eq!(
        rig.core.request(get("/courses/c1")).await.unwrap().status,
        404
    );
    rig.mock.down();
    assert!(
        rig.core.request(get("/courses/c1")).await.is_err(),
        "nothing left to show"
    );
}

// --- the queue ------------------------------------------------------------------------------------

#[tokio::test]
async fn a_draft_saved_offline_is_shown_now_and_sent_later_with_its_key() {
    let rig = rig().await;
    sign_in(&rig, "u1").await;
    rig.mock.answer(200, json!({"draft": null, "attempts": []}));
    rig.core
        .request(get("/assignments/a1/submission"))
        .await
        .unwrap();
    rig.mock.down();

    let content = json!({"v": 1, "blocks": []});
    let reply = rig
        .core
        .request(queued(
            "PUT",
            DRAFT,
            Some(json!({"content": content})),
            "key-1",
        ))
        .await
        .unwrap();
    assert!(reply.queued);
    assert_eq!(reply.body.as_ref().unwrap()["content"], content);
    assert!(
        reply.body.as_ref().unwrap()["updated_at"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );
    let state = rig.core.state();
    assert_eq!((state.online, state.pending, state.failed), (false, 1, 0));
    assert_eq!(state.queued, [DRAFT]);

    // A reload shows the text from the copy, and the copy asks for a full answer when it is online.
    let shown = rig
        .core
        .request(get("/assignments/a1/submission"))
        .await
        .unwrap();
    assert!(shown.from_cache);
    assert_eq!(shown.body.unwrap()["draft"]["content"], content);

    rig.mock.up().await;
    rig.mock.answer(
        200,
        json!({"content": content, "updated_at": "2026-10-19T10:00:00Z"}),
    );
    assert_eq!(rig.core.deliver_due().await, 1);
    let sent: Vec<_> = rig
        .mock
        .seen()
        .into_iter()
        .filter(|s| s.method == "PUT")
        .collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].header("idempotency-key"), Some("key-1"));
    assert_eq!(sent[0].header("authorization"), Some("Bearer T0KEN"));
    assert_eq!(
        serde_json::from_str::<Value>(&sent[0].body).unwrap(),
        json!({"content": content})
    );
    let state = rig.core.state();
    assert_eq!((state.online, state.pending, state.sent), (true, 0, 1));
    assert!(state.last_ok.is_some());
}

#[tokio::test]
async fn a_write_that_gets_through_is_not_queued() {
    let rig = rig().await;
    sign_in(&rig, "u1").await;
    rig.mock.answer(201, json!({"attempt": 1}));
    let reply = rig
        .core
        .request(queued("POST", SUBMIT, None, "key-2"))
        .await
        .unwrap();
    assert_eq!(
        (reply.status, reply.queued, reply.body),
        (201, false, Some(json!({"attempt": 1})))
    );
    assert_eq!(rig.core.state().pending, 0);
}

#[tokio::test]
async fn the_queue_keeps_its_order_and_a_new_write_does_not_overtake_it() {
    let rig = rig().await;
    sign_in(&rig, "u1").await;
    rig.mock.down();
    rig.core
        .request(queued("PUT", DRAFT, Some(json!({"content": 1})), "k1"))
        .await
        .unwrap();
    rig.core
        .request(queued("POST", SUBMIT, None, "k2"))
        .await
        .unwrap();
    rig.mock.up().await;
    // The server is back, but writes are waiting: a new one joins them instead of passing them.
    let third = rig
        .core
        .request(queued(
            "POST",
            "/assignments/a2/submission/submit",
            None,
            "k3",
        ))
        .await
        .unwrap();
    assert!(third.queued);
    assert_eq!(
        rig.mock
            .seen()
            .iter()
            .filter(|s| s.method != "POST" || s.path != "/auth/login")
            .count(),
        0
    );

    assert_eq!(rig.core.deliver_due().await, 3);
    let order: Vec<_> = rig
        .mock
        .seen()
        .into_iter()
        .skip(1)
        .map(|s| (s.method, s.path))
        .collect();
    assert_eq!(
        order,
        [
            ("PUT".to_owned(), DRAFT.to_owned()),
            ("POST".to_owned(), SUBMIT.to_owned()),
            (
                "POST".to_owned(),
                "/assignments/a2/submission/submit".to_owned()
            )
        ]
    );
}

#[tokio::test]
async fn what_the_server_refuses_is_shown_not_retried_and_does_not_hold_up_the_rest() {
    let rig = rig().await;
    sign_in(&rig, "u1").await;
    rig.mock.down();
    rig.core
        .request(queued("POST", SUBMIT, None, "k1"))
        .await
        .unwrap();
    rig.core
        .request(queued(
            "POST",
            "/assignments/a2/submission/submit",
            None,
            "k2",
        ))
        .await
        .unwrap();
    rig.mock.up().await;
    rig.mock.answer(
        409,
        json!({"title": "Conflict", "detail": "the submission window is closed"}),
    );
    rig.mock.answer(201, json!({"attempt": 1}));
    rig.core.deliver_due().await;

    let rows = rig.core.outbox().unwrap();
    assert_eq!(rows.len(), 1, "the second was sent and left the queue");
    assert_eq!(
        (rows[0].state.as_str(), rows[0].status),
        ("failed", Some(409))
    );
    assert!(
        rows[0]
            .problem
            .as_deref()
            .unwrap()
            .contains("the submission window is closed")
    );
    let state = rig.core.state();
    assert_eq!((state.pending, state.failed, state.sent), (0, 1, 1));

    rig.core.deliver_due().await;
    assert_eq!(
        rig.mock.seen().len(),
        3,
        "a refusal is final: nothing is sent again"
    );
    rig.core.discard(rows[0].id).unwrap();
    assert_eq!(rig.core.state().failed, 0);
}

#[tokio::test]
async fn a_busy_server_is_asked_again_later_with_the_same_key() {
    let rig = rig().await;
    sign_in(&rig, "u1").await;
    rig.mock.down();
    rig.core
        .request(queued("POST", SUBMIT, None, "k1"))
        .await
        .unwrap();
    rig.mock.up().await;
    rig.mock.answer_with(
        503,
        vec![("retry-after", "30".into())],
        json!({"title": "busy"}),
    );
    assert_eq!(rig.core.deliver_due().await, 1);

    let row = &rig.core.outbox().unwrap()[0];
    assert_eq!((row.state.as_str(), row.attempts), ("pending", 1));
    assert!(
        row.next_at >= now_ms() / 1000 + 25,
        "the server asked for 30 s"
    );
    assert_eq!(rig.core.deliver_due().await, 0, "not due yet");

    // The person presses "try now".
    rig.mock.answer(201, json!({}));
    rig.core.sync_now().await;
    assert_eq!(rig.core.deliver_due().await, 1);
    let posts: Vec<_> = rig
        .mock
        .seen()
        .into_iter()
        .filter(|s| s.path == SUBMIT)
        .collect();
    assert_eq!(posts.len(), 2);
    assert_eq!(
        posts[0].header("idempotency-key"),
        posts[1].header("idempotency-key")
    );
    assert_eq!(rig.core.state().pending, 0);
}

#[tokio::test]
async fn an_ended_session_holds_the_queue_until_the_person_signs_in_again() {
    let rig = rig().await;
    sign_in(&rig, "u1").await;
    rig.mock.down();
    rig.core
        .request(queued("POST", SUBMIT, None, "k1"))
        .await
        .unwrap();
    rig.mock.up().await;
    rig.mock.answer(401, json!({"title": "Unauthorized"}));
    rig.core.deliver_due().await;
    let state = rig.core.state();
    assert_eq!(
        (state.paused, state.pending, state.failed),
        (true, 1, 0),
        "nothing is lost"
    );
    assert_eq!(rig.core.deliver_due().await, 0);

    sign_in(&rig, "u1").await;
    assert!(!rig.core.state().paused);
    rig.mock.answer(201, json!({}));
    assert_eq!(rig.core.deliver_due().await, 1);
    assert_eq!(rig.core.state().pending, 0);
}

// --- the session ----------------------------------------------------------------------------------

#[tokio::test]
async fn the_token_stays_in_the_app_and_the_window_gets_the_user() {
    let rig = rig().await;
    sign_in(&rig, "u1").await;
    assert_eq!(rig.token.get().as_deref(), Some("T0KEN"));
    rig.mock.answer(200, json!({"id": "u1"}));
    rig.core.request(get("/me")).await.unwrap();
    assert_eq!(
        rig.mock.seen().last().unwrap().header("authorization"),
        Some("Bearer T0KEN")
    );
}

#[tokio::test]
async fn another_person_signing_in_starts_with_nothing_of_the_last_one() {
    let rig = rig().await;
    sign_in(&rig, "u1").await;
    rig.mock.answer(200, json!({"n": 1}));
    rig.core.request(get("/courses/c1")).await.unwrap();
    rig.mock.down();
    rig.core
        .request(queued("POST", SUBMIT, None, "k1"))
        .await
        .unwrap();
    rig.mock.up().await;

    // The same person again: what they read and had not sent is still theirs.
    sign_in(&rig, "u1").await;
    assert_eq!(rig.core.state().pending, 1);
    assert!(rig.core.store.cache_get("/courses/c1").unwrap().is_some());

    sign_in(&rig, "u2").await;
    assert_eq!(rig.core.state().pending, 0);
    assert!(rig.core.store.cache_get("/courses/c1").unwrap().is_none());
}

#[tokio::test]
async fn signing_out_clears_the_token_and_the_store_even_when_the_server_is_gone() {
    let rig = rig().await;
    sign_in(&rig, "u1").await;
    rig.mock.answer(200, json!({"n": 1}));
    rig.core.request(get("/courses/c1")).await.unwrap();
    rig.mock.down();
    let reply = rig
        .core
        .request(ApiRequest {
            method: "POST".into(),
            path: "/auth/logout".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(reply.status, 204);
    assert!(rig.token.get().is_none());
    assert!(rig.core.store.cache_get("/courses/c1").unwrap().is_none());
    assert!(
        !rig.events.lock().unwrap().is_empty(),
        "the window was told"
    );
}

#[tokio::test]
async fn what_is_not_a_path_inside_the_api_is_refused() {
    let rig = rig().await;
    assert!(rig.core.request(get("/../etc/passwd")).await.is_err());
    assert!(
        rig.core
            .request(ApiRequest {
                method: "TRACE".into(),
                path: "/me".into(),
                ..Default::default()
            })
            .await
            .is_err()
    );
    assert!(
        rig.core
            .request(ApiRequest {
                method: "POST".into(),
                path: "/x".into(),
                queue: true,
                ..Default::default()
            })
            .await
            .is_err(),
        "a queued write needs a key"
    );
    assert!(rig.mock.seen().is_empty());
}

// --- against a real server (not run by default) ---------------------------------------------------

/// Checks the app against a running `vumlo-server` with the demo data: its `ETag`, its `Idempotency-Key`, the
/// session, and the store surviving a restart.
/// `VUMLO_E2E_API=http://127.0.0.1:8080/api/v1 VUMLO_E2E_EMAIL=... VUMLO_E2E_PASSWORD=... cargo test -- --ignored`
#[tokio::test]
#[ignore = "needs a running server; see the comment"]
async fn the_app_works_against_a_real_server_and_its_store_outlives_a_restart() {
    let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"));
    let (api_url, email, password) = (
        var("VUMLO_E2E_API"),
        var("VUMLO_E2E_EMAIL"),
        var("VUMLO_E2E_PASSWORD"),
    );
    let dir = std::env::temp_dir().join(format!("vumlo-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("offline.sqlite3");
    let token = Arc::new(MemoryToken::default());
    let open = |url: &str| {
        Core::new(
            Store::open(&file).unwrap(),
            url.to_owned(),
            token.clone(),
            Box::new(|_| {}),
        )
    };
    let call = |method: &str, path: &str, body: Option<Value>, key: Option<&str>| ApiRequest {
        method: method.into(),
        path: path.into(),
        body,
        queue: key.is_some(),
        idem_key: key.map(Into::into),
    };

    // Signed in and reading: the server's tag comes back as a 304 and the copy is confirmed.
    let core = open(&api_url);
    let login = core
        .request(call(
            "POST",
            "/auth/login",
            Some(json!({"email": email, "password": password, "client": "app"})),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(login.status, 200, "{:?}", login.body);
    assert!(token.get().is_some() && login.body.as_ref().unwrap().get("token").is_none());
    let first = core.request(get("/me/courses?limit=100")).await.unwrap();
    let course = first.body.as_ref().unwrap()["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let tag = core
        .store
        .cache_get("/me/courses?limit=100")
        .unwrap()
        .unwrap()
        .etag;
    assert!(
        tag.as_deref().is_some_and(|t| t.starts_with("W/\"")),
        "{tag:?}"
    );
    let second = core.request(get("/me/courses?limit=100")).await.unwrap();
    assert_eq!(
        (second.status, second.from_cache, second.body),
        (200, false, first.body.clone())
    );
    let content = format!("/courses/{course}/content");
    core.request(get(&content)).await.unwrap();

    // The same key twice is one module: the server replays the first answer.
    let modules = format!("/courses/{course}/modules");
    let title = format!("Offline test {}", std::process::id());
    let create = |key: &str| call("POST", &modules, Some(json!({"title": title})), Some(key));
    let one = core.request(create("e2e-key-1")).await.unwrap();
    let again = core.request(create("e2e-key-1")).await.unwrap();
    assert_eq!(
        (one.status, one.queued, again.status),
        (201, false, 201),
        "{:?}",
        one.body
    );
    assert_eq!(one.body, again.body, "the same module");
    drop(core);

    // "Restart" with the server unreachable: what was read is still there, and a write waits in the queue.
    let offline = open("http://127.0.0.1:9");
    let kept = offline.request(get(&content)).await.unwrap();
    assert!(kept.from_cache && kept.cached_at.is_some());
    let queued = offline.request(create("e2e-key-2")).await.unwrap();
    assert!(queued.queued);
    assert_eq!(offline.state().pending, 1);
    drop(offline);

    // The server is back (a third start): the queue is still there and goes out with its key, once.
    let online = open(&api_url);
    assert_eq!(online.state().pending, 1);
    assert_eq!(online.deliver_due().await, 1);
    assert_eq!(
        (
            online.state().pending,
            online.state().failed,
            online.state().sent
        ),
        (0, 0, 1)
    );
    online.store.cache_delete(&content).unwrap();
    let tree = online.request(get(&content)).await.unwrap().body.unwrap();
    let count = tree["modules"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["title"] == title)
        .count();
    assert_eq!(
        count, 2,
        "one for key 1 and one for key 2, none for the replays"
    );

    let out = online
        .request(call("POST", "/auth/logout", None, None))
        .await
        .unwrap();
    assert_eq!(out.status, 204);
    std::fs::remove_dir_all(&dir).unwrap();
}
