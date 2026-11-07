//! ADR-0070: the Buddy. A fake OpenAI-compatible server on loopback stands in for the model: it
//! replays scripted answers and records every request it gets, so each test can check what the
//! model was actually sent. The module is compiled in by the feature `ai_agent`.
#![cfg(feature = "ai_agent")]

mod common;

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use ::common::Role;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use common::{Account, Env, db_test, migrate_modules, seed_and_login, spawn_app, test_settings};
use serde_json::{Value, json};
use server::modules;
use sqlx::PgPool;
use uuid::Uuid;
use vumlo_config::{ModuleEntry, Settings, resolve_modules};

const KEY: &str = "sk-test-key";

// --- the fake model -----------------------------------------------------------------------------

struct Seen {
    authorization: Option<String>,
    body: Value,
}

#[derive(Clone, Default)]
struct Fake {
    /// `None` is an HTTP 500. When the script runs out the model answers with plain text.
    script: Arc<Mutex<VecDeque<Option<Value>>>>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Fake {
    fn seen(&self) -> Vec<Value> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|s| s.body.clone())
            .collect()
    }
}

fn text(content: &str) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": content}}]})
}

fn tool(name: &str, arguments: Value) -> Value {
    json!({"choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [
        {"id": "c1", "type": "function",
         "function": {"name": name, "arguments": arguments.to_string()}}]}}]})
}

async fn completions(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    fake.seen.lock().unwrap().push(Seen {
        authorization: headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
        body,
    });
    match fake.script.lock().unwrap().pop_front() {
        Some(Some(answer)) => (StatusCode::OK, Json(answer)).into_response(),
        Some(None) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        None => (StatusCode::OK, Json(text("(the script ended)"))).into_response(),
    }
}

/// Starts the fake model; returns its endpoint (`http://127.0.0.1:port/v1`).
async fn fake_model(script: Vec<Option<Value>>) -> (String, Fake) {
    let fake = Fake {
        script: Arc::new(Mutex::new(script.into())),
        seen: Arc::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = Router::new()
        .route("/v1/chat/completions", post(completions))
        .with_state(fake.clone());
    tokio::spawn(async move { axum::serve(listener, app).await });
    (format!("http://127.0.0.1:{port}/v1"), fake)
}

fn answers(script: Vec<Value>) -> Vec<Option<Value>> {
    script.into_iter().map(Some).collect()
}

// --- the server ---------------------------------------------------------------------------------

fn settings(endpoint: &str, extra: &[(&str, Value)]) -> Settings {
    let mut table: BTreeMap<String, Value> = BTreeMap::from([
        ("enabled".to_owned(), json!(true)),
        ("allow_insecure_http".to_owned(), json!(true)),
        ("endpoint".to_owned(), json!(endpoint)),
        ("model".to_owned(), json!("test-model")),
    ]);
    table.extend(extra.iter().map(|(k, v)| ((*k).to_owned(), v.clone())));
    let mut settings = test_settings();
    settings.modules = BTreeMap::from([("ai_agent".to_owned(), ModuleEntry::Table(table))]);
    let env = BTreeMap::from([("VUMLO_AI_AGENT_API_KEY".to_owned(), KEY.to_owned())]);
    settings.module_state = resolve_modules(&settings.modules, &env, &modules::specs())
        .expect("the ai_agent settings resolve");
    settings
}

async fn env(pool: PgPool, endpoint: &str, extra: &[(&str, Value)]) -> Env {
    migrate_modules(&pool).await;
    Env::with_settings(pool, settings(endpoint, extra)).await
}

async fn send(env: &Env, who: &Account, message: &str) -> common::Resp {
    env.call(
        "POST",
        &who.token,
        "/modules/ai_agent/messages",
        Some(json!({"text": message, "context": {"page": "/courses"}})),
    )
    .await
}

/// What the model was sent as tool results, parsed, in order.
fn tool_results(request: &Value) -> Vec<Value> {
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "tool")
        .map(|m| serde_json::from_str(m["content"].as_str().unwrap()).unwrap())
        .collect()
}

fn tool_names(request: &Value) -> Vec<String> {
    request["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .map(|t| t["function"]["name"].as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

// --- tests --------------------------------------------------------------------------------------

#[tokio::test]
async fn a_teacher_asks_and_the_buddy_looks_it_up_as_that_teacher() {
    db_test(async |pool| {
        let (endpoint, fake) = fake_model(answers(vec![
            tool("teaching_overview", json!({})),
            text("You teach one course."),
        ]))
        .await;
        let env = env(pool, &endpoint, &[]).await;
        let teacher = env.user("t@x.edu", Role::Teacher).await;
        let student = env.user("s@x.edu", Role::Student).await;
        env.course(&teacher, &[&student]).await;

        let reply = send(&env, &teacher, "What do I teach?").await;
        assert_eq!(
            reply.status,
            200,
            "{}",
            String::from_utf8_lossy(&reply.body)
        );
        let body = reply.json();
        assert_eq!(body["user"]["content"], "What do I teach?");
        assert_eq!(body["assistant"]["content"], "You teach one course.");

        let seen = fake.seen();
        assert_eq!(seen.len(), 2, "one call for the tool, one for the answer");
        assert_eq!(
            tool_names(&seen[0]),
            [
                "teaching_overview",
                "course_gradebook",
                "course_content",
                "set_grade",
                "publish_grades",
                "set_deadline",
                "create_module",
                "create_text_item",
                "create_assignment",
                "create_quiz",
            ]
        );
        let system = seen[0]["messages"][0]["content"].as_str().unwrap();
        assert!(
            system.contains("teacher") && system.contains("/courses"),
            "{system}"
        );
        assert_eq!(
            fake.seen.lock().unwrap()[0].authorization.as_deref(),
            Some(&*format!("Bearer {KEY}")),
            "the key goes to the model and nowhere else"
        );
        let result = &tool_results(&seen[1])[0];
        assert_eq!(result["courses"][0]["title"], "C", "{result}");

        // Only the visible chat is kept: not the tool call, not its result.
        let chat = env
            .ok("GET", &teacher.token, "/modules/ai_agent/chat", None, 200)
            .await;
        let roles: Vec<_> = chat["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["role"].clone())
            .collect();
        assert_eq!(roles, [json!("user"), json!("assistant")]);
        assert!(!chat.to_string().contains("courses"), "{chat}");
    })
    .await;
}

#[tokio::test]
async fn a_students_buddy_never_reads_a_draft_grade_and_is_offered_no_teaching_tools() {
    db_test(async |pool| {
        let (endpoint, fake) = fake_model(answers(vec![
            tool("my_work", json!({})),
            tool("teaching_overview", json!({})),
            text("done"),
        ]))
        .await;
        let env = env(
            pool,
            &endpoint,
            &[("roles", json!("student,teacher,admin"))],
        )
        .await;
        let teacher = env.user("t@x.edu", Role::Teacher).await;
        let student = env.user("s@x.edu", Role::Student).await;
        let (course, module) = env.course(&teacher, &[&student]).await;
        let assignment = env.assignment(&teacher, module, "20").await;

        // A published grade, then a newer draft the student must never see (ADR-0028).
        let grade = format!("/assignments/{assignment}/grades/{}", student.id);
        env.ok(
            "PUT",
            &teacher.token,
            &grade,
            Some(json!({"score": "11.11", "feedback": "first"})),
            200,
        )
        .await;
        let publish = format!("/assignments/{assignment}/grades/publish");
        env.ok(
            "POST",
            &teacher.token,
            &publish,
            Some(json!({"students": "all"})),
            200,
        )
        .await;
        env.ok(
            "PUT",
            &teacher.token,
            &grade,
            Some(json!({"score": "17.77", "feedback": "DRAFT-ONLY-NOTE"})),
            200,
        )
        .await;

        // The model asks for my_work, then for a tool a student was never offered.
        let reply = send(&env, &student, "What is due?").await;
        assert_eq!(
            reply.status,
            200,
            "{}",
            String::from_utf8_lossy(&reply.body)
        );
        fake.script.lock().unwrap().clear();
        fake.script.lock().unwrap().extend(answers(vec![
            tool("my_grades", json!({"course_id": course})),
            text("ok"),
        ]));
        let reply = send(&env, &student, "How am I doing?").await;
        assert_eq!(
            reply.status,
            200,
            "{}",
            String::from_utf8_lossy(&reply.body)
        );

        let seen = fake.seen();
        assert_eq!(
            tool_names(&seen[0]),
            [
                "my_work",
                "my_grades",
                "assignment_info",
                "save_draft",
                "submit_work"
            ]
        );

        // 1: my_work lists the assignment with no draft anything.
        let work = &tool_results(&seen[1])[0];
        assert_eq!(work["assignments"][0]["title"], "A", "{work}");
        // 2: a teaching tool asked for by a student's model does not run.
        assert_eq!(tool_results(&seen[2])[1], json!({"error": "unknown tool"}));
        // 3: my_grades has the published value and nothing of the draft.
        let grades = seen.last().unwrap();
        let wire = serde_json::to_string(grades).unwrap();
        assert!(wire.contains("11.11") && wire.contains("first"), "{wire}");
        assert!(
            !wire.contains("17.77") && !wire.contains("DRAFT-ONLY-NOTE"),
            "{wire}"
        );
        assert!(!wire.contains("draft_"), "{wire}");
    })
    .await;
}

#[tokio::test]
async fn the_buddy_does_not_exist_for_a_role_the_admin_did_not_switch_on() {
    db_test(async |pool| {
        let (endpoint, fake) = fake_model(vec![]).await;
        let env = env(pool, &endpoint, &[]).await; // default roles: teacher, admin
        let student = env.user("s@x.edu", Role::Student).await;
        let teacher = env.user("t@x.edu", Role::Teacher).await;

        env.call("GET", &student.token, "/modules/ai_agent/chat", None)
            .await
            .problem(404);
        send(&env, &student, "hello").await.problem(404);
        env.call("DELETE", &student.token, "/modules/ai_agent/chat", None)
            .await
            .problem(404);
        assert!(fake.seen().is_empty(), "the model was never asked");

        env.ok("GET", &teacher.token, "/modules/ai_agent/chat", None, 200)
            .await;
        env.http
            .get("/modules/ai_agent/chat")
            .send()
            .await
            .problem(401);
    })
    .await;
}

#[tokio::test]
async fn the_notice_and_the_input_limits() {
    db_test(async |pool| {
        let (endpoint, _) = fake_model(vec![]).await;
        let env = env(
            pool,
            &endpoint,
            &[("notice", json!("Messages go to Example AI."))],
        )
        .await;
        let teacher = env.user("t@x.edu", Role::Teacher).await;

        let chat = env
            .ok("GET", &teacher.token, "/modules/ai_agent/chat", None, 200)
            .await;
        assert_eq!(chat["notice"], "Messages go to Example AI.");

        for body in [
            json!({"text": "   "}),
            json!({"text": "x".repeat(4001)}),
            json!({"text": "hi", "context": {"page": "p".repeat(201)}}),
        ] {
            let r = env
                .call(
                    "POST",
                    &teacher.token,
                    "/modules/ai_agent/messages",
                    Some(body),
                )
                .await;
            let problem = r.problem(422);
            assert!(!problem.errors.is_empty(), "{problem:?}");
        }
    })
    .await;
}

#[tokio::test]
async fn the_daily_cap_stops_the_third_message_and_a_failed_model_call_costs_nothing() {
    db_test(async |pool| {
        let (endpoint, fake) = fake_model(vec![None, Some(text("one")), Some(text("two"))]).await;
        let env = env(pool, &endpoint, &[("daily_cap", json!(2))]).await;
        let teacher = env.user("t@x.edu", Role::Teacher).await;

        // The model is down: 503, and the message is given back.
        send(&env, &teacher, "first try").await.problem(503);
        assert_eq!(send(&env, &teacher, "second try").await.status, 200);
        assert_eq!(send(&env, &teacher, "third try").await.status, 200);
        assert_eq!(fake.seen().len(), 3);

        // Two answered messages used the cap; the next never reaches the model.
        send(&env, &teacher, "one too many").await.problem(429);
        assert_eq!(fake.seen().len(), 3);

        // Clearing the chat does not reset the cap.
        env.ok(
            "DELETE",
            &teacher.token,
            "/modules/ai_agent/chat",
            None,
            204,
        )
        .await;
        send(&env, &teacher, "after clearing").await.problem(429);
    })
    .await;
}

#[tokio::test]
async fn a_chat_is_private_to_its_user_and_can_be_cleared() {
    db_test(async |pool| {
        let (endpoint, fake) = fake_model(vec![]).await;
        let env = env(pool, &endpoint, &[]).await;
        let anna = env.user("anna@x.edu", Role::Teacher).await;
        let boris = env.user("boris@x.edu", Role::Admin).await;

        assert_eq!(send(&env, &anna, "ANNA-SECRET").await.status, 200);
        assert_eq!(send(&env, &boris, "hello").await.status, 200);

        // Boris's model never read Anna's words, and his chat holds only his own.
        let to_boris = fake.seen().pop().unwrap();
        assert!(!to_boris.to_string().contains("ANNA-SECRET"), "{to_boris}");
        let chat = env
            .ok("GET", &boris.token, "/modules/ai_agent/chat", None, 200)
            .await;
        assert!(!chat.to_string().contains("ANNA-SECRET"), "{chat}");
        assert_eq!(chat["messages"].as_array().unwrap().len(), 2);

        // The earlier turns of one's own chat go back to the model.
        assert_eq!(send(&env, &anna, "and again").await.status, 200);
        let again = fake.seen().pop().unwrap();
        assert!(again.to_string().contains("ANNA-SECRET"), "{again}");

        // Clearing deletes only the caller's chat.
        env.ok("DELETE", &anna.token, "/modules/ai_agent/chat", None, 204)
            .await;
        let anna_chat = env
            .ok("GET", &anna.token, "/modules/ai_agent/chat", None, 200)
            .await;
        assert_eq!(anna_chat["messages"], json!([]));
        let boris_chat = env
            .ok("GET", &boris.token, "/modules/ai_agent/chat", None, 200)
            .await;
        assert_eq!(boris_chat["messages"].as_array().unwrap().len(), 2);
    })
    .await;
}

#[tokio::test]
async fn the_worker_deletes_chats_older_than_the_retention() {
    db_test(async |pool| {
        let (endpoint, _) = fake_model(vec![]).await;
        migrate_modules(&pool).await;
        let (_, state) = spawn_app(pool.clone(), settings(&endpoint, &[])).await;
        let user = Uuid::now_v7();
        // Runtime queries: the schema of an optional module is not part of the core's `.sqlx`.
        for (content, age) in [("old", "40 days"), ("recent", "1 day")] {
            sqlx::query(
                "INSERT INTO mod_ai_agent.messages (id, user_id, role, content, created_at)
                 VALUES ($1, $2, 'user', $3, now() - $4::interval)",
            )
            .bind(Uuid::now_v7())
            .bind(user)
            .bind(content)
            .bind(age)
            .execute(&pool)
            .await
            .unwrap();
        }

        let _workers = modules::spawn_workers(&state);
        common::wait_until(|| async {
            let left: Vec<String> =
                sqlx::query_scalar("SELECT content FROM mod_ai_agent.messages WHERE user_id = $1")
                    .bind(user)
                    .fetch_all(&pool)
                    .await
                    .unwrap();
            left == ["recent"]
        })
        .await;
    })
    .await;
}

#[tokio::test]
async fn the_module_is_in_the_openapi_document_and_the_login_flow_is_the_cores() {
    db_test(async |pool| {
        let (endpoint, _) = fake_model(vec![]).await;
        let env = env(pool, &endpoint, &[]).await;
        let doc = env.http.get("/openapi.json").send().await.json();
        for path in [
            "/api/v1/modules/ai_agent/chat",
            "/api/v1/modules/ai_agent/messages",
        ] {
            assert!(doc["paths"].get(path).is_some(), "{path} is missing: {doc}");
        }
        let (_, token) = seed_and_login(&env.pool, &env.http, "a@x.edu", Role::Admin).await;
        env.ok("GET", &token, "/modules/ai_agent/chat", None, 200)
            .await;
    })
    .await;
}

// --- proposals: the Buddy prepares, only the user's confirm changes anything ------------------------

/// A published course with a teacher, a student and one 20-point assignment, and a Buddy for all roles.
struct Class {
    env: Env,
    fake: Fake,
    teacher: Account,
    student: Account,
    course: Uuid,
    module: Uuid,
    assignment: Uuid,
}

async fn class(pool: PgPool) -> Class {
    let (endpoint, fake) = fake_model(vec![]).await;
    let env = env(
        pool,
        &endpoint,
        &[("roles", json!("student,teacher,admin"))],
    )
    .await;
    let teacher = env.user("t@x.edu", Role::Teacher).await;
    let student = env.user("s@x.edu", Role::Student).await;
    let (course, module) = env.course(&teacher, &[&student]).await;
    let assignment = env.assignment(&teacher, module, "20").await;
    Class {
        env,
        fake,
        teacher,
        student,
        course,
        module,
        assignment,
    }
}

impl Class {
    /// The teacher asks; the model answers with `call` and then with words. Returns the response body.
    async fn propose(&self, call: Value) -> Value {
        self.fake
            .script
            .lock()
            .unwrap()
            .extend(answers(vec![call, text("Prepared.")]));
        let reply = send(&self.env, &self.teacher, "do it").await;
        assert_eq!(
            reply.status,
            200,
            "{}",
            String::from_utf8_lossy(&reply.body)
        );
        reply.json()
    }

    async fn confirm(&self, who: &Account, proposal: &str) -> u16 {
        let path = format!("/modules/ai_agent/proposals/{proposal}/confirm");
        self.env.call("POST", &who.token, &path, None).await.status
    }

    fn set_grade(&self, score: Value, feedback: Value) -> Value {
        tool(
            "set_grade",
            json!({
                "assignment_id": self.assignment, "student_id": self.student.id,
                "score": score, "feedback": feedback,
            }),
        )
    }

    async fn draft(&self) -> Vec<(Option<String>, Option<String>)> {
        sqlx::query!(
            "SELECT draft_score::text AS score, draft_feedback FROM gradebook.grades WHERE assignment_id = $1",
            self.assignment
        )
        .fetch_all(&self.env.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.score, r.draft_feedback))
        .collect()
    }

    async fn students_grades(&self) -> Value {
        let path = format!("/courses/{}/my-grades", self.course);
        self.env
            .ok("GET", &self.student.token, &path, None, 200)
            .await
    }

    async fn pending(&self, who: &Account) -> usize {
        let chat = self
            .env
            .ok("GET", &who.token, "/modules/ai_agent/chat", None, 200)
            .await;
        chat["proposals"].as_array().unwrap().len()
    }
}

#[tokio::test]
async fn a_grade_is_only_proposed_until_the_user_confirms_and_then_it_is_a_draft_the_student_cannot_see()
 {
    db_test(async |pool| {
        let c = class(pool).await;
        let body = c.propose(c.set_grade(json!(8), json!("Good work"))).await;

        // The card: facts the service read as the teacher, not words of the model.
        let proposals = body["proposals"].as_array().unwrap();
        assert_eq!(proposals.len(), 1, "{body}");
        let p = &proposals[0];
        assert_eq!(p["tool"], "set_grade");
        assert_eq!(p["facts"]["assignment"], "A");
        assert_eq!(p["facts"]["score"], "8");
        assert_eq!(p["facts"]["max_points"], "20.00");
        assert_eq!(p["facts"]["feedback"], "Good work");
        assert!(!p["facts"]["student"].as_str().unwrap().is_empty());

        // The model was told it is not done, and nothing changed: not even a trace of the dry run.
        let second_request = &c.fake.seen()[1];
        assert_eq!(tool_results(second_request)[0]["proposed"], true);
        assert!(
            c.draft().await.is_empty(),
            "nothing is saved before the confirm"
        );
        let audited = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM gradebook.grade_history WHERE assignment_id = $1"#,
            c.assignment
        )
        .fetch_one(&c.env.pool)
        .await
        .unwrap();
        assert_eq!(audited, 0);
        assert_eq!(c.pending(&c.teacher).await, 1, "it waits in the chat");

        // The confirm saves a draft, with the core's own audit row, once.
        let id = p["id"].as_str().unwrap();
        assert_eq!(c.confirm(&c.teacher, id).await, 200);
        assert_eq!(
            c.draft().await,
            [(Some("8.00".to_owned()), Some("Good work".to_owned()))]
        );
        let audited = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM gradebook.grade_history WHERE assignment_id = $1"#,
            c.assignment
        )
        .fetch_one(&c.env.pool)
        .await
        .unwrap();
        assert_eq!(audited, 1);
        assert_eq!(
            c.students_grades().await,
            json!({"items": []}),
            "a draft is never shown to the student"
        );
        assert_eq!(
            c.confirm(&c.teacher, id).await,
            409,
            "a second confirm does nothing"
        );
        assert_eq!(c.draft().await.len(), 1);
        assert_eq!(c.pending(&c.teacher).await, 0);
    })
    .await;
}

#[tokio::test]
async fn publishing_and_a_deadline_are_proposed_and_confirmed_the_same_way() {
    db_test(async |pool| {
        let c = class(pool).await;
        let grade = c.propose(c.set_grade(json!("11"), json!(null))).await;
        c.confirm(&c.teacher, grade["proposals"][0]["id"].as_str().unwrap())
            .await;

        let publish = c
            .propose(tool(
                "publish_grades",
                json!({"assignment_id": c.assignment}),
            ))
            .await;
        assert_eq!(publish["proposals"][0]["tool"], "publish_grades");
        assert_eq!(
            c.students_grades().await,
            json!({"items": []}),
            "not before the confirm"
        );
        assert_eq!(
            c.confirm(&c.teacher, publish["proposals"][0]["id"].as_str().unwrap())
                .await,
            200
        );
        let mine = c.students_grades().await;
        assert_eq!(mine["items"][0]["score"], "11.00", "{mine}");

        let due = "2031-03-01T10:00:00Z";
        let deadline = c
            .propose(tool(
                "set_deadline",
                json!({"assignment_id": c.assignment, "due_at": due}),
            ))
            .await;
        assert_eq!(
            deadline["proposals"][0]["facts"]["due_at"],
            "2031-03-01T10:00:00+00:00"
        );
        let item = |c: &Class| format!("/items/{}", c.assignment);
        let before = c
            .env
            .ok("GET", &c.teacher.token, &item(&c), None, 200)
            .await;
        assert_eq!(before["assignment"]["due_at"], Value::Null);
        assert_eq!(
            c.confirm(&c.teacher, deadline["proposals"][0]["id"].as_str().unwrap())
                .await,
            200
        );
        let after = c
            .env
            .ok("GET", &c.teacher.token, &item(&c), None, 200)
            .await;
        assert_eq!(
            after["assignment"]["due_at"], "2031-03-01T10:00:00Z",
            "{after}"
        );
        assert_eq!(
            after["assignment"]["max_points"], "20.00",
            "only the deadline changed"
        );

        // And it can be taken away again.
        let clear = c
            .propose(tool(
                "set_deadline",
                json!({"assignment_id": c.assignment, "due_at": null}),
            ))
            .await;
        assert_eq!(
            c.confirm(&c.teacher, clear["proposals"][0]["id"].as_str().unwrap())
                .await,
            200
        );
        let gone = c
            .env
            .ok("GET", &c.teacher.token, &item(&c), None, 200)
            .await;
        assert_eq!(gone["assignment"]["due_at"], Value::Null);
    })
    .await;
}

#[tokio::test]
async fn only_the_owner_can_confirm_and_two_confirms_at_once_make_one_change() {
    db_test(async |pool| {
        let c = class(pool).await;
        let other = c.env.user("t2@x.edu", Role::Teacher).await;
        let body = c.propose(c.set_grade(json!(5), json!(null))).await;
        let id = body["proposals"][0]["id"].as_str().unwrap().to_owned();

        assert_eq!(
            c.confirm(&other, &id).await,
            409,
            "somebody else's proposal is nothing to them"
        );
        assert!(c.draft().await.is_empty());
        assert_eq!(
            c.confirm(&c.student, &id).await,
            409,
            "nor is it a student's"
        );

        let (a, b) = tokio::join!(c.confirm(&c.teacher, &id), c.confirm(&c.teacher, &id));
        let mut both = [a, b];
        both.sort();
        assert_eq!(both, [200, 409]);
        assert_eq!(c.draft().await.len(), 1);
        let history = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM gradebook.grade_history WHERE assignment_id = $1"#,
            c.assignment
        )
        .fetch_one(&c.env.pool)
        .await
        .unwrap();
        assert_eq!(history, 1, "the change was made once");
    })
    .await;
}

#[tokio::test]
async fn a_rejected_cleared_or_old_proposal_can_no_longer_be_confirmed() {
    db_test(async |pool| {
        let c = class(pool).await;
        let reject = |id: String| {
            let path = format!("/modules/ai_agent/proposals/{id}/reject");
            let (env, token) = (&c.env, c.teacher.token.clone());
            async move { env.call("POST", &token, &path, None).await.status }
        };

        let first = c.propose(c.set_grade(json!(1), json!(null))).await["proposals"][0]["id"].as_str().unwrap().to_owned();
        assert_eq!(reject(first.clone()).await, 204);
        assert_eq!(reject(first.clone()).await, 409);
        assert_eq!(c.confirm(&c.teacher, &first).await, 409);

        let second = c.propose(c.set_grade(json!(2), json!(null))).await["proposals"][0]["id"].as_str().unwrap().to_owned();
        c.env.ok("DELETE", &c.teacher.token, "/modules/ai_agent/chat", None, 204).await;
        assert_eq!(c.confirm(&c.teacher, &second).await, 409, "clearing the chat drops what it proposed");

        let third = c.propose(c.set_grade(json!(3), json!(null))).await["proposals"][0]["id"].as_str().unwrap().to_owned();
        sqlx::query("UPDATE mod_ai_agent.proposals SET created_at = now() - interval '2 days' WHERE id = $1::uuid")
            .bind(&third)
            .execute(&c.env.pool)
            .await
            .unwrap();
        assert_eq!(c.pending(&c.teacher).await, 0, "an old proposal is not offered");
        assert_eq!(c.confirm(&c.teacher, &third).await, 409);
        assert!(c.draft().await.is_empty(), "none of the three changed anything");
    })
    .await;
}

#[tokio::test]
async fn a_change_the_core_would_refuse_is_never_proposed_and_the_model_is_told_why() {
    db_test(async |pool| {
        let c = class(pool).await;

        // More points than the assignment is worth: the service's own limit, found by the dry run.
        let over = c.propose(c.set_grade(json!("25"), json!(null))).await;
        assert_eq!(over["proposals"], json!([]), "{over}");
        let told = tool_results(&c.fake.seen()[1])[0]["error"].as_str().unwrap().to_owned();
        assert!(told.contains("score"), "{told}");

        // A student who is not on the course, an item that is not an assignment, a missing field.
        let stranger = c.env.user("x@x.edu", Role::Student).await;
        let call = tool("set_grade", json!({"assignment_id": c.assignment, "student_id": stranger.id, "score": "5", "feedback": null}));
        assert_eq!(c.propose(call).await["proposals"], json!([]));
        let call = tool("set_deadline", json!({"assignment_id": Uuid::now_v7(), "due_at": null}));
        assert_eq!(c.propose(call).await["proposals"], json!([]));
        let call = tool("set_grade", json!({"assignment_id": c.assignment, "student_id": c.student.id, "score": "5"}));
        assert_eq!(c.propose(call).await["proposals"], json!([]));
        let seen = c.fake.seen();
        assert!(tool_results(seen.last().unwrap())[0]["error"].as_str().unwrap().contains("feedback is required"));

        assert_eq!(c.pending(&c.teacher).await, 0);
        assert!(c.draft().await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn a_change_the_core_refuses_at_confirm_is_rolled_back_and_not_offered_again() {
    db_test(async |pool| {
        let c = class(pool).await;
        let body = c.propose(c.set_grade(json!(7), json!("fine"))).await;
        let id = body["proposals"][0]["id"].as_str().unwrap().to_owned();

        // Between the proposal and the confirm the course is archived, which is read-only (ADR-0048).
        c.env
            .ok(
                "POST",
                &c.teacher.token,
                &format!("/courses/{}/archive", c.course),
                None,
                200,
            )
            .await;
        let path = format!("/modules/ai_agent/proposals/{id}/confirm");
        let refused = c.env.call("POST", &c.teacher.token, &path, None).await;
        refused.problem(409);

        assert!(
            c.draft().await.is_empty(),
            "the refused change left nothing"
        );
        assert_eq!(c.pending(&c.teacher).await, 0);
        assert_eq!(c.confirm(&c.teacher, &id).await, 409);
        // Runtime query: the schema of an optional module is not part of the core's `.sqlx`.
        let status: String =
            sqlx::query_scalar("SELECT status FROM mod_ai_agent.proposals WHERE id = $1::uuid")
                .bind(&id)
                .fetch_one(&c.env.pool)
                .await
                .unwrap();
        assert_eq!(status, "failed");
    })
    .await;
}

#[tokio::test]
async fn what_an_admin_confirms_is_audited_by_the_core_like_any_admin_change() {
    db_test(async |pool| {
        let c = class(pool).await;
        let admin = c.env.user("a@x.edu", Role::Admin).await;
        c.fake.script.lock().unwrap().extend(answers(vec![
            c.set_grade(json!(9), json!(null)),
            text("Prepared."),
        ]));
        let reply = send(&c.env, &admin, "grade it").await;
        assert_eq!(
            reply.status,
            200,
            "{}",
            String::from_utf8_lossy(&reply.body)
        );
        let id = reply.json()["proposals"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();

        let audited = || async {
            sqlx::query_scalar!(
                r#"SELECT count(*) AS "n!" FROM events.audit_log WHERE action = 'grade.save'"#
            )
            .fetch_one(&c.env.pool)
            .await
            .unwrap()
        };
        assert_eq!(audited().await, 0, "proposing writes nothing");
        assert_eq!(c.confirm(&admin, &id).await, 200);
        assert_eq!(
            audited().await,
            1,
            "the core's own audit row, in the confirm's transaction"
        );
        assert_eq!(c.draft().await.len(), 1);
    })
    .await;
}

// --- proposals that create content: always drafts, checked by the core's own rules ---------------------

fn fractions_quiz(c: &Class) -> Value {
    tool(
        "create_quiz",
        json!({
            "course_id": c.course, "module_id": c.module, "title": "Fractions quiz",
            "text": "Answer all questions.",
            "questions": [
                {"prompt": "What is 1/2 + 1/4?", "type": "single_choice", "points": 2,
                 "options": ["1/4", "3/4", "1"], "correct": [1]},
                {"prompt": "Simplify 2/4", "type": "short_text", "points": 1, "accepted": ["1/2"]},
                {"prompt": "Explain why 1/3 is smaller than 1/2", "type": "long_text", "points": 2}
            ]
        }),
    )
}

impl Class {
    async fn items_named(&self, title: &str) -> Vec<(Uuid, String, String)> {
        sqlx::query!(
            "SELECT id, kind, status FROM content.items WHERE course_id = $1 AND title = $2",
            self.course,
            title
        )
        .fetch_all(&self.env.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.id, r.kind, r.status))
        .collect()
    }
}

#[tokio::test]
async fn a_quiz_is_proposed_with_its_answer_key_visible_and_created_only_as_a_draft_on_confirm() {
    db_test(async |pool| {
        let c = class(pool).await;
        let body = c.propose(fractions_quiz(&c)).await;
        let p = &body["proposals"][0];
        assert_eq!(p["tool"], "create_quiz", "{body}");
        let facts = &p["facts"];
        assert_eq!(
            (facts["course"].as_str(), facts["module"].as_str()),
            (Some("C"), Some("M"))
        );
        assert_eq!(
            (facts["questions"].as_str(), facts["max_points"].as_str()),
            (Some("3"), Some("5"))
        );
        let preview = facts["preview"].as_str().unwrap();
        assert!(
            preview.contains("b) 3/4 ✓"),
            "the card shows which answer is the right one: {preview}"
        );
        assert!(preview.contains("= 1/2"), "{preview}");

        assert!(
            c.items_named("Fractions quiz").await.is_empty(),
            "nothing exists before the confirm"
        );
        assert_eq!(c.confirm(&c.teacher, p["id"].as_str().unwrap()).await, 200);

        let created = c.items_named("Fractions quiz").await;
        assert_eq!(created.len(), 1);
        let (id, kind, status) = &created[0];
        assert_eq!((kind.as_str(), status.as_str()), ("assignment", "draft"));
        let item = c
            .env
            .ok("GET", &c.teacher.token, &format!("/items/{id}"), None, 200)
            .await;
        assert_eq!(item["assignment"]["type"], "quiz", "{item}");
        assert_eq!(item["assignment"]["max_points"], "5.00");
        assert_eq!(
            item["assignment"]["config"]["questions"][0]["correct"],
            json!(["b"])
        );

        // Students see nothing of it: it is a draft, and the key never leaves the staff.
        let tree = c
            .env
            .ok(
                "GET",
                &c.student.token,
                &format!("/courses/{}/content", c.course),
                None,
                200,
            )
            .await;
        assert!(!tree.to_string().contains("Fractions quiz"), "{tree}");
        c.env
            .call("GET", &c.student.token, &format!("/items/{id}"), None)
            .await
            .problem(404);
    })
    .await;
}

#[tokio::test]
async fn a_module_a_page_and_an_assignment_are_created_as_drafts_one_confirm_at_a_time() {
    db_test(async |pool| {
        let c = class(pool).await;
        let module = c.propose(tool("create_module", json!({"course_id": c.course, "title": "Unit 2"}))).await;
        assert_eq!(module["proposals"][0]["facts"]["course"], "C");
        let count = |c: &Class| {
            let (pool, course) = (c.env.pool.clone(), c.course);
            async move {
                sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM content.modules WHERE course_id = $1"#, course)
                    .fetch_one(&pool)
                    .await
                    .unwrap()
            }
        };
        let before = count(&c).await;
        assert_eq!(c.confirm(&c.teacher, module["proposals"][0]["id"].as_str().unwrap()).await, 200);
        assert_eq!(count(&c).await, before + 1);
        let status = sqlx::query_scalar!(r#"SELECT status AS "s!" FROM content.modules WHERE course_id = $1 AND title = 'Unit 2'"#, c.course)
            .fetch_one(&c.env.pool)
            .await
            .unwrap();
        assert_eq!(status, "draft");

        let page = c.propose(tool("create_text_item", json!({
            "course_id": c.course, "module_id": c.module, "title": "Reading notes",
            "text": "Fractions name parts of a whole.\n\nThe bottom number is the denominator."
        }))).await;
        assert_eq!(page["proposals"][0]["facts"]["module"], "M");
        assert_eq!(c.confirm(&c.teacher, page["proposals"][0]["id"].as_str().unwrap()).await, 200);
        assert_eq!(c.items_named("Reading notes").await[0].1, "text");

        let essay = c.propose(tool("create_assignment", json!({
            "course_id": c.course, "module_id": c.module, "title": "Essay", "max_points": 10,
            "text": "Write a page about fractions."
        }))).await;
        assert_eq!(essay["proposals"][0]["facts"]["max_points"], "10");
        assert_eq!(c.confirm(&c.teacher, essay["proposals"][0]["id"].as_str().unwrap()).await, 200);
        let made = c.items_named("Essay").await;
        assert_eq!((made[0].1.as_str(), made[0].2.as_str()), ("assignment", "draft"));
    })
    .await;
}

#[tokio::test]
async fn content_the_core_would_refuse_is_never_proposed() {
    db_test(async |pool| {
        let c = class(pool).await;
        let told = |c: &Class| tool_results(c.fake.seen().last().unwrap())[0]["error"].as_str().unwrap().to_owned();

        // The grading scale of the institution applies: the regular assignments of a course add up to
        // at most 100 points, and this course already has one worth 20.
        let too_much = c.propose(tool("create_assignment", json!({
            "course_id": c.course, "module_id": c.module, "title": "Too big", "max_points": 90
        }))).await;
        assert_eq!(too_much["proposals"], json!([]), "{too_much}");
        assert!(!told(&c).is_empty());

        // A module that is not in that course, an answer key that points at no option, a missing field.
        let elsewhere = c.propose(tool("create_text_item", json!({
            "course_id": c.course, "module_id": Uuid::now_v7(), "title": "Lost", "text": "x"
        }))).await;
        assert_eq!(elsewhere["proposals"], json!([]));
        assert!(told(&c).contains("module"), "{}", told(&c));

        let bad_key = c.propose(tool("create_quiz", json!({
            "course_id": c.course, "module_id": c.module, "title": "Bad key",
            "questions": [{"prompt": "?", "type": "single_choice", "points": 1, "options": ["a", "b"], "correct": [5]}]
        }))).await;
        assert_eq!(bad_key["proposals"], json!([]));
        assert!(told(&c).contains("correct"), "{}", told(&c));

        let no_points = c.propose(tool("create_assignment", json!({
            "course_id": c.course, "module_id": c.module, "title": "No points"
        }))).await;
        assert_eq!(no_points["proposals"], json!([]));
        assert!(told(&c).contains("max_points"), "{}", told(&c));

        for title in ["Too big", "Lost", "Bad key", "No points"] {
            assert!(c.items_named(title).await.is_empty(), "{title}");
        }
        assert_eq!(c.pending(&c.teacher).await, 0);
    })
    .await;
}

#[tokio::test]
async fn the_model_can_find_where_to_put_content_and_a_student_cannot_ask_for_it() {
    db_test(async |pool| {
        let c = class(pool).await;
        c.fake.script.lock().unwrap().extend(answers(vec![
            tool("course_content", json!({"course_id": c.course})),
            text("Here it is."),
        ]));
        assert_eq!(
            send(&c.env, &c.teacher, "what is in my course?")
                .await
                .status,
            200
        );
        let tree = &tool_results(&c.fake.seen()[1])[0];
        let module = &tree["modules"][0];
        assert_eq!(
            (module["id"].as_str(), module["title"].as_str()),
            (Some(c.module.to_string().as_str()), Some("M"))
        );
        assert_eq!(module["items"][0]["title"], "A");
        assert_eq!(module["items"][0]["kind"], "assignment");

        // A student's model that asks anyway is told there is no such tool.
        c.fake.script.lock().unwrap().extend(answers(vec![
            tool("course_content", json!({"course_id": c.course})),
            text("ok"),
        ]));
        assert_eq!(
            send(&c.env, &c.student, "show me everything").await.status,
            200
        );
        let last = c.fake.seen().pop().unwrap();
        assert_eq!(tool_results(&last)[0], json!({"error": "unknown tool"}));
    })
    .await;
}

// --- a student's own work: the Buddy relays words the student wrote and never writes them -------------

const ESSAY: &str = "Fractions are parts of a whole.\nOne half is bigger than one third.";

impl Class {
    /// The student writes `message`; the model answers with `call` and then with words.
    async fn student_says(&self, message: &str, call: Value) -> Value {
        self.fake
            .script
            .lock()
            .unwrap()
            .extend(answers(vec![call, text("Prepared.")]));
        let reply = send(&self.env, &self.student, message).await;
        assert_eq!(
            reply.status,
            200,
            "{}",
            String::from_utf8_lossy(&reply.body)
        );
        reply.json()
    }

    fn save_draft(&self, words: &str) -> Value {
        tool(
            "save_draft",
            json!({"assignment_id": self.assignment, "text": words}),
        )
    }

    /// The student's rows for the assignment: status and content as text.
    async fn work(&self) -> Vec<(String, String)> {
        sqlx::query!(
            r#"SELECT status, content::text AS "content!" FROM assessment.submissions
               WHERE assignment_id = $1 AND student_id = $2 ORDER BY id"#,
            self.assignment,
            self.student.id
        )
        .fetch_all(&self.env.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.status, r.content))
        .collect()
    }

    fn told_last(&self) -> String {
        let seen = self.fake.seen();
        tool_results(seen.last().unwrap())[0]["error"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }
}

#[tokio::test]
async fn a_student_saves_a_draft_of_their_own_words_only_when_they_confirm() {
    db_test(async |pool| {
        let c = class(pool).await;
        // The words come back with other line breaks than the student typed: that is no difference.
        let body = c
            .student_says(
                &format!("Save this as my answer:\n{ESSAY}"),
                c.save_draft(&ESSAY.replace('\n', " ")),
            )
            .await;
        let p = &body["proposals"][0];
        assert_eq!(
            (p["tool"].as_str(), p["facts"]["assignment"].as_str()),
            (Some("save_draft"), Some("A")),
            "{body}"
        );
        assert!(
            p["facts"]["preview"]
                .as_str()
                .unwrap()
                .contains("One half is bigger")
        );
        assert!(
            c.work().await.is_empty(),
            "nothing is saved before the confirm"
        );

        let id = p["id"].as_str().unwrap();
        assert_eq!(c.confirm(&c.student, id).await, 200);
        let work = c.work().await;
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].0, "draft");
        assert!(
            work[0].1.contains("Fractions are parts of a whole."),
            "{}",
            work[0].1
        );

        // Saving again replaces the draft: the core's rule, not a second row.
        let again = c
            .student_says(
                "Better:\nOne half is bigger than one third.",
                c.save_draft("One half is bigger than one third."),
            )
            .await;
        assert_eq!(
            c.confirm(&c.student, again["proposals"][0]["id"].as_str().unwrap())
                .await,
            200
        );
        let work = c.work().await;
        assert_eq!(work.len(), 1);
        assert!(!work[0].1.contains("parts of a whole"), "{}", work[0].1);
    })
    .await;
}

#[tokio::test]
async fn text_the_model_wrote_never_becomes_a_draft_even_when_the_student_says_yes_to_it() {
    db_test(async |pool| {
        let c = class(pool).await;

        // The model invents an answer for a student who asked for one.
        let invented = c
            .student_says(
                "write my essay for me",
                c.save_draft("A wonderful essay that the model wrote."),
            )
            .await;
        assert_eq!(invented["proposals"], json!([]), "{invented}");
        assert!(
            c.told_last().contains("never write it for them"),
            "{}",
            c.told_last()
        );

        // The model suggests words in its own answer, and the student only agrees.
        c.fake.script.lock().unwrap().extend(answers(vec![text(
            "An idea: fractions divide a whole into equal parts.",
        )]));
        assert_eq!(
            send(&c.env, &c.student, "give me an idea").await.status,
            200
        );
        let agreed = c
            .student_says(
                "yes, save that",
                c.save_draft("fractions divide a whole into equal parts."),
            )
            .await;
        assert_eq!(
            agreed["proposals"],
            json!([]),
            "words from the model's answer are not the student's: {agreed}"
        );

        // Nor can it improve what the student wrote.
        let improved = c
            .student_says(
                "Here: fractions are parts of a whole",
                c.save_draft("Fractions are wonderful parts of a whole."),
            )
            .await;
        assert_eq!(improved["proposals"], json!([]));

        assert!(c.work().await.is_empty());
        assert_eq!(c.pending(&c.student).await, 0);
    })
    .await;
}

#[tokio::test]
async fn submitting_needs_a_saved_draft_and_is_final_only_after_the_students_confirm() {
    db_test(async |pool| {
        let c = class(pool).await;
        let submit = || tool("submit_work", json!({"assignment_id": c.assignment}));

        let none = c.student_says("submit it", submit()).await;
        assert_eq!(none["proposals"], json!([]));
        assert!(
            c.told_last().contains("no saved draft"),
            "{}",
            c.told_last()
        );

        let saved = c.student_says(ESSAY, c.save_draft(ESSAY)).await;
        assert_eq!(
            c.confirm(&c.student, saved["proposals"][0]["id"].as_str().unwrap())
                .await,
            200
        );

        let body = c.student_says("ok, submit my answer", submit()).await;
        let p = &body["proposals"][0];
        assert_eq!(p["tool"], "submit_work", "{body}");
        assert!(
            p["facts"]["preview"]
                .as_str()
                .unwrap()
                .contains("One half is bigger"),
            "the card shows what will be handed in"
        );
        assert_eq!(
            c.work().await[0].0,
            "draft",
            "still a draft before the confirm"
        );

        let id = p["id"].as_str().unwrap();
        assert_eq!(c.confirm(&c.student, id).await, 200);
        assert_eq!(c.work().await[0].0, "submitted");
        assert_eq!(c.confirm(&c.student, id).await, 409, "it is handed in once");

        // The draft is gone, so there is nothing left to submit.
        let again = c.student_says("submit again", submit()).await;
        assert_eq!(again["proposals"], json!([]));
        assert_eq!(c.work().await.len(), 1);
    })
    .await;
}

#[tokio::test]
async fn the_cores_deadline_applies_to_what_a_student_proposes() {
    db_test(async |pool| {
        let c = class(pool).await;
        // The window closed a minute ago, with no late work taken.
        let past = (chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339();
        let path = format!("/items/{}", c.assignment);
        c.env
            .ok(
                "PATCH",
                &c.teacher.token,
                &path,
                Some(json!({"due_at": past, "on_deadline": "keep_draft", "accept_late": false})),
                200,
            )
            .await;

        let body = c.student_says(ESSAY, c.save_draft(ESSAY)).await;
        assert_eq!(body["proposals"], json!([]), "{body}");
        assert!(!c.told_last().is_empty());
        assert!(c.work().await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn a_coach_reads_the_instructions_of_a_text_assignment_but_never_the_questions_of_a_quiz() {
    db_test(async |pool| {
        let c = class(pool).await;
        let doc = |text: &str| json!({"v": 1, "blocks": [{"id": "p1", "type": "paragraph", "content": [{"type": "text", "text": text}]}]});
        let create = |body: Value| {
            let (env, token, module) = (&c.env, c.teacher.token.clone(), c.module);
            async move {
                let item = env.ok("POST", &token, &format!("/modules/{module}/items"), Some(body), 201).await["id"].as_str().unwrap().to_owned();
                env.ok("POST", &token, &format!("/items/{item}/publish"), None, 200).await;
                item
            }
        };
        let essay = create(json!({"kind": "assignment", "title": "Essay", "body": doc("Describe a fraction in your own words."),
            "assignment": {"type": "text", "max_points": "5"}})).await;
        let quiz = create(json!({"kind": "assignment", "title": "Quiz", "assignment": {"type": "quiz", "max_points": "1", "config": {
            "v": 1, "questions": [{"id": "q1", "type": "long_text", "points": "1", "prompt": doc("SECRET QUESTION")}]}}})).await;

        for (id, title) in [(&essay, "Essay"), (&quiz, "Quiz")] {
            c.fake.script.lock().unwrap().extend(answers(vec![tool("assignment_info", json!({"assignment_id": id})), text("ok")]));
            assert_eq!(send(&c.env, &c.student, "what is this about?").await.status, 200);
            let info = tool_results(&c.fake.seen().pop().unwrap()).pop().unwrap();
            assert_eq!(info["title"], title, "{info}");
            if title == "Essay" {
                assert_eq!(info["instructions"], "Describe a fraction in your own words.");
            } else {
                assert_eq!(info["type"], "quiz");
                assert_eq!(info["instructions"], Value::Null);
                assert!(!info.to_string().contains("SECRET QUESTION"), "{info}");
            }
        }
    })
    .await;
}

// --- people and the organisation: an admin's pack ---------------------------------------------------

async fn admin_proposes(c: &Class, admin: &Account, call: Value) -> Value {
    c.fake
        .script
        .lock()
        .unwrap()
        .extend(answers(vec![call, text("Prepared.")]));
    let reply = send(&c.env, admin, "do it").await;
    assert_eq!(
        reply.status,
        200,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    reply.json()
}

/// What the model was last told by its tool: the result of the call it made.
fn last_tool_result(c: &Class) -> Value {
    tool_results(&c.fake.seen().pop().unwrap()).pop().unwrap()
}

#[tokio::test]
async fn an_admin_finds_people_and_groups_and_only_an_admin_is_offered_those_tools() {
    db_test(async |pool| {
        let c = class(pool).await;
        let admin = c.env.user("a@x.edu", Role::Admin).await;
        let group = c
            .env
            .ok(
                "POST",
                &admin.token,
                "/groups",
                Some(json!({"name": "7B"})),
                201,
            )
            .await;
        c.env
            .ok(
                "POST",
                &admin.token,
                "/terms",
                Some(json!({"name": "Autumn", "starts_on": "2026-09-01", "ends_on": "2027-01-31"})),
                201,
            )
            .await;

        // By email, by name, and narrowed by role.
        for (query, role, expected) in [
            ("s@x.edu", Value::Null, 1),
            ("x.edu", json!("teacher"), 1),
            ("x.edu", Value::Null, 3),
            ("nobody", Value::Null, 0),
        ] {
            c.fake.script.lock().unwrap().extend(answers(vec![
                tool("find_people", json!({"query": query, "role": role})),
                text("ok"),
            ]));
            assert_eq!(send(&c.env, &admin, "who?").await.status, 200);
            let people = last_tool_result(&c)["people"].as_array().unwrap().clone();
            assert_eq!(people.len(), expected, "{query} {role}: {people:?}");
        }
        c.fake.script.lock().unwrap().extend(answers(vec![
            tool("find_people", json!({"query": "Seed", "role": null})),
            text("ok"),
        ]));
        send(&c.env, &admin, "who is Seed?").await;
        let by_name = last_tool_result(&c);
        assert!(by_name.get("error").is_none(), "{by_name}");

        c.fake.script.lock().unwrap().extend(answers(vec![
            tool("org_overview", json!({"query": "7"})),
            text("ok"),
        ]));
        send(&c.env, &admin, "what is there?").await;
        let overview = last_tool_result(&c);
        assert_eq!(overview["terms"][0]["name"], "Autumn", "{overview}");
        assert_eq!(overview["groups"][0]["id"], group["id"], "{overview}");

        // A teacher and a student are not offered them, and asking for them anyway gets "unknown tool".
        for (who, message) in [(&c.teacher, "who?"), (&c.student, "who?")] {
            c.fake.script.lock().unwrap().extend(answers(vec![
                tool("find_people", json!({"query": "x"})),
                text("ok"),
            ]));
            assert_eq!(send(&c.env, who, message).await.status, 200);
            let last = c.fake.seen().pop().unwrap();
            assert_eq!(tool_results(&last).pop().unwrap()["error"], "unknown tool");
            let offered = tool_names(&c.fake.seen()[c.fake.seen().len() - 2]);
            assert!(
                !offered.iter().any(|n| [
                    "find_people",
                    "org_overview",
                    "create_term",
                    "create_group",
                    "add_group_members",
                    "reset_password"
                ]
                .contains(&n.as_str())),
                "{offered:?}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn a_term_a_group_and_its_students_are_proposed_and_made_only_on_confirm_with_the_cores_audit()
 {
    db_test(async |pool| {
        let c = class(pool).await;
        let admin = c.env.user("a@x.edu", Role::Admin).await;
        let terms = || async {
            sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM org.terms"#).fetch_one(&c.env.pool).await.unwrap()
        };
        let groups = || async {
            sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM org.groups"#).fetch_one(&c.env.pool).await.unwrap()
        };
        let members = || async {
            sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM org.group_members"#).fetch_one(&c.env.pool).await.unwrap()
        };
        let audited = |action: &'static str| {
            let pool = c.env.pool.clone();
            async move {
                sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM events.audit_log WHERE action = $1"#, action).fetch_one(&pool).await.unwrap()
            }
        };

        let term = admin_proposes(&c, &admin, tool("create_term", json!({"name": " Spring 2027 ", "starts_on": "2027-02-01", "ends_on": "2027-06-30"}))).await;
        assert_eq!(term["proposals"][0]["facts"]["name"], "Spring 2027");
        assert_eq!(term["proposals"][0]["facts"]["ends_on"], "2027-06-30");
        assert_eq!(terms().await, 0, "nothing before the confirm");
        assert_eq!(c.confirm(&admin, term["proposals"][0]["id"].as_str().unwrap()).await, 200);
        assert_eq!((terms().await, audited("term.create").await), (1, 1));

        let group = admin_proposes(&c, &admin, tool("create_group", json!({"name": "8A"}))).await;
        assert_eq!(group["proposals"][0]["facts"]["name"], "8A");
        assert_eq!(groups().await, 0);
        assert_eq!(c.confirm(&admin, group["proposals"][0]["id"].as_str().unwrap()).await, 200);
        let group_id = sqlx::query_scalar!(r#"SELECT id AS "id!" FROM org.groups WHERE name = '8A'"#).fetch_one(&c.env.pool).await.unwrap();

        // The card names the group and the people from the service, the model's ids only point at them.
        let second = c.env.user("s2@x.edu", Role::Student).await;
        let add = admin_proposes(&c, &admin, tool("add_group_members", json!({"group_id": group_id, "user_ids": [c.student.id, second.id, c.student.id]}))).await;
        let facts = &add["proposals"][0]["facts"];
        assert_eq!(facts["group"], "8A");
        assert_eq!(facts["count"], "2", "a repeated id counts once");
        assert!(facts["people"].as_str().unwrap().contains("s2@x.edu"), "{facts}");
        assert_eq!(members().await, 0);
        assert_eq!(c.confirm(&admin, add["proposals"][0]["id"].as_str().unwrap()).await, 200);
        assert_eq!((members().await, audited("group.member.add").await), (2, 1));

        // The core's rules apply before a card exists: only students join a group, and a name that is taken is refused.
        let teacher_in = admin_proposes(&c, &admin, tool("add_group_members", json!({"group_id": group_id, "user_ids": [c.teacher.id]}))).await;
        assert!(teacher_in["proposals"].as_array().unwrap().is_empty(), "{teacher_in}");
        assert!(last_tool_result(&c)["error"].as_str().unwrap().contains("user_ids"), "{}", last_tool_result(&c));
        let taken = admin_proposes(&c, &admin, tool("create_group", json!({"name": "8A"}))).await;
        assert!(taken["proposals"].as_array().unwrap().is_empty());
        let nobody = admin_proposes(&c, &admin, tool("add_group_members", json!({"group_id": group_id, "user_ids": [Uuid::now_v7()]}))).await;
        assert!(nobody["proposals"].as_array().unwrap().is_empty());
        let none = admin_proposes(&c, &admin, tool("add_group_members", json!({"group_id": group_id, "user_ids": []}))).await;
        assert!(none["proposals"].as_array().unwrap().is_empty());

        // A teacher who is sent the same call gets nothing: the tool does not exist for them.
        c.fake.script.lock().unwrap().extend(answers(vec![tool("create_group", json!({"name": "9Z"})), text("ok")]));
        let reply = send(&c.env, &c.teacher, "make a group").await.json();
        assert!(reply["proposals"].as_array().unwrap().is_empty());
        assert_eq!(groups().await, 1);
    })
    .await;
}

#[tokio::test]
async fn a_password_reset_shows_the_temporary_password_once_to_the_admin_and_to_nobody_else() {
    db_test(async |pool| {
        let c = class(pool).await;
        let admin = c.env.user("a@x.edu", Role::Admin).await;
        let audited = || async {
            sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM events.audit_log WHERE action = 'user.password_reset'"#).fetch_one(&c.env.pool).await.unwrap()
        };

        let reset = admin_proposes(&c, &admin, tool("reset_password", json!({"user_id": c.student.id}))).await;
        let proposal = &reset["proposals"][0];
        assert_eq!(proposal["facts"]["email"], "s@x.edu");
        assert_eq!(proposal["facts"]["role"], "student");
        assert!(proposal["facts"].get("password").is_none() && proposal["facts"].get("temporary_password").is_none());
        assert_eq!(c.env.ok("GET", &c.student.token, "/me", None, 200).await["email"], "s@x.edu");
        assert_eq!(audited().await, 0, "the dry run leaves nothing, not even a signed-out student");

        // Only the confirm resets it, and the password is in that response alone.
        let id = proposal["id"].as_str().unwrap();
        let path = format!("/modules/ai_agent/proposals/{id}/confirm");
        assert_eq!(c.env.call("POST", &c.teacher.token, &path, None).await.status, 409, "not theirs");
        let done = c.env.call("POST", &admin.token, &path, None).await;
        assert_eq!(done.status, 200, "{}", String::from_utf8_lossy(&done.body));
        assert_eq!(done.header("cache-control"), "no-store");
        let temporary = done.json()["temporary_password"].as_str().unwrap().to_owned();
        assert!(temporary.len() >= 20, "a real password, not a placeholder");
        assert_eq!(audited().await, 1);
        assert_eq!(c.env.call("GET", &c.student.token, "/me", None).await.status, 401, "signed out everywhere");
        let token = common::login_app(&c.env.http, "s@x.edu", &temporary).await;
        assert_eq!(c.env.ok("GET", &token, "/me", None, 200).await["must_change_password"], true);
        assert_eq!(c.env.call("POST", &admin.token, &path, None).await.status, 409, "once");
        assert_eq!(audited().await, 1);

        // It is nowhere the model, the chat or the stored proposals could show it.
        let everything = format!(
            "{} {}",
            serde_json::to_string(&c.fake.seen()).unwrap(),
            sqlx::query_scalar!(r#"SELECT coalesce(string_agg(content, ' '), '') AS "t!" FROM mod_ai_agent.messages"#).fetch_one(&c.env.pool).await.unwrap(),
        );
        assert!(!everything.contains(&temporary));
        let stored = sqlx::query_scalar!(r#"SELECT coalesce(string_agg(args || facts, ' '), '') AS "t!" FROM mod_ai_agent.proposals"#).fetch_one(&c.env.pool).await.unwrap();
        assert!(!stored.contains(&temporary));

        // Other confirms answer without a password, and a person's own account is not reset from here.
        let group = admin_proposes(&c, &admin, tool("create_group", json!({"name": "9C"}))).await;
        let plain = c.env.call("POST", &admin.token, &format!("/modules/ai_agent/proposals/{}/confirm", group["proposals"][0]["id"].as_str().unwrap()), None).await;
        assert_eq!(plain.header("cache-control"), "no-store");
        assert_eq!(plain.json(), json!({"temporary_password": null}));
        let own = admin_proposes(&c, &admin, tool("reset_password", json!({"user_id": admin.id}))).await;
        assert!(own["proposals"].as_array().unwrap().is_empty());
        assert!(last_tool_result(&c)["error"].as_str().unwrap().contains("own account"), "{}", last_tool_result(&c));
        let nobody = admin_proposes(&c, &admin, tool("reset_password", json!({"user_id": Uuid::now_v7()}))).await;
        assert!(nobody["proposals"].as_array().unwrap().is_empty());

        // A teacher cannot ask for it.
        c.fake.script.lock().unwrap().extend(answers(vec![tool("reset_password", json!({"user_id": c.student.id})), text("ok")]));
        let reply = send(&c.env, &c.teacher, "reset the student").await.json();
        assert!(reply["proposals"].as_array().unwrap().is_empty());
        assert_eq!(audited().await, 1);
    })
    .await;
}
