//! Drafts, attempts, deadlines and who reads what (ADR-0011, ADR-0035), over HTTP, plus the
//! constraints of the table against a writer that skips the service.

mod common;

use chrono::{DateTime, Duration, Utc};
use common::{Http, Resp, create_course, id_of, publish_course, put_member, seed_and_login};
use serde_json::{Value, json};
use server::identity::Role;
use server::settings::{EditorSettings, ProfileSettings, Settings};
use sqlx::PgPool;
use uuid::Uuid;

struct User {
    id: Uuid,
    token: String,
}

async fn user(pool: &PgPool, http: &Http, email: &str, role: Role) -> User {
    let (id, token) = seed_and_login(pool, http, email, role).await;
    User { id, token }
}

/// A published course with an owner, an assistant (a student at the institution), two students, a
/// teacher who is no member and an admin; and a published module.
struct World {
    http: Http,
    pool: PgPool,
    owner: User,
    assistant: User,
    student: User,
    other: User,
    stranger: User,
    admin: User,
    course: Uuid,
    module: Uuid,
}

async fn world(pool: PgPool) -> World {
    world_from(pool, common::test_settings()).await
}

async fn world_from(pool: PgPool, settings: Settings) -> World {
    let http = Http::new(&common::spawn_with(pool.clone(), settings).await);
    let owner = user(&pool, &http, "owner@x.edu", Role::Teacher).await;
    let assistant = user(&pool, &http, "assistant@x.edu", Role::Student).await;
    let student = user(&pool, &http, "student@x.edu", Role::Student).await;
    let other = user(&pool, &http, "other@x.edu", Role::Student).await;
    let stranger = user(&pool, &http, "stranger@x.edu", Role::Teacher).await;
    let admin = user(&pool, &http, "admin@x.edu", Role::Admin).await;
    let course = create_course(&http, &owner.token, "Algebra 7-A").await;
    put_member(&http, &owner.token, course, assistant.id, "assistant").await;
    put_member(&http, &owner.token, course, student.id, "student").await;
    put_member(&http, &owner.token, course, other.id, "student").await;
    publish_course(&http, &owner.token, course).await;
    let module = id_of(&json_of(
        http.post(&format!("/courses/{course}/modules"))
            .bearer(&owner.token)
            .json(&json!({"title": "M"}))
            .send()
            .await,
        201,
    ));
    json_of(
        http.post(&format!("/modules/{module}/publish"))
            .bearer(&owner.token)
            .json(&json!({"cascade": true}))
            .send()
            .await,
        200,
    );
    World {
        http,
        pool,
        owner,
        assistant,
        student,
        other,
        stranger,
        admin,
        course,
        module,
    }
}

fn json_of(response: Resp, status: u16) -> Value {
    assert_eq!(
        response.status,
        status,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    response.json()
}

async fn call(w: &World, method: &str, token: &str, path: &str, body: Option<Value>) -> Resp {
    let request = match method {
        "GET" => w.http.get(path),
        "POST" => w.http.post(path),
        "PUT" => w.http.put(path),
        "PATCH" => w.http.patch(path),
        "DELETE" => w.http.delete(path),
        other => panic!("method {other}"),
    }
    .bearer(token);
    match body {
        Some(body) => request.json(&body).send().await,
        None => request.send().await,
    }
}

// --- documents ---------------------------------------------------------------------------------

fn doc(blocks: Value) -> Value {
    json!({"v": 1, "blocks": blocks, "attachments": []})
}

fn paragraph(id: &str, text: &str) -> Value {
    json!({"id": id, "type": "paragraph", "content": [{"type": "text", "text": text}]})
}

fn essay(text: &str) -> Value {
    doc(json!([paragraph("p1", text)]))
}

// --- assignments -------------------------------------------------------------------------------

fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339()
}

/// The settings of an assignment due at `due`, taking late work until `late_until` when `late`.
fn deadline(due: DateTime<Utc>, late: bool, late_until: Option<DateTime<Utc>>) -> Value {
    let mut settings = json!({
        "due_at": stamp(due),
        "on_deadline": "keep_draft",
        "accept_late": late,
    });
    if let Some(until) = late_until {
        settings["late_until"] = json!(stamp(until));
    }
    settings
}

/// Creates a published `text` assignment with `settings` and `config` merged in and returns its id.
async fn assignment_with(w: &World, settings: Value, config: Value) -> Uuid {
    let mut assignment = json!({"type": "text", "max_points": "10", "config": config});
    for (key, value) in settings.as_object().unwrap() {
        assignment[key] = value.clone();
    }
    let item = id_of(&json_of(
        call(
            w,
            "POST",
            &w.owner.token,
            &format!("/modules/{}/items", w.module),
            Some(json!({"kind": "assignment", "title": "Essay", "assignment": assignment})),
        )
        .await,
        201,
    ));
    json_of(
        call(
            w,
            "POST",
            &w.owner.token,
            &format!("/items/{item}/publish"),
            None,
        )
        .await,
        200,
    );
    item
}

async fn assignment(w: &World) -> Uuid {
    assignment_with(w, json!({}), json!({})).await
}

fn draft_path(id: Uuid) -> String {
    format!("/assignments/{id}/submission/draft")
}

fn submission_path(id: Uuid) -> String {
    format!("/assignments/{id}/submission")
}

async fn save(w: &World, token: &str, id: Uuid, content: Value) -> Resp {
    call(
        w,
        "PUT",
        token,
        &draft_path(id),
        Some(json!({"content": content})),
    )
    .await
}

async fn submit(w: &World, token: &str, id: Uuid) -> Resp {
    call(
        w,
        "POST",
        token,
        &format!("{}/submit", submission_path(id)),
        None,
    )
    .await
}

async fn mine(w: &World, token: &str, id: Uuid) -> Value {
    json_of(call(w, "GET", token, &submission_path(id), None).await, 200)
}

/// Saves `text` as the draft and submits it; returns the attempt.
async fn turn_in(w: &World, token: &str, id: Uuid, text: &str) -> Value {
    json_of(save(w, token, id, essay(text)).await, 200);
    json_of(submit(w, token, id).await, 201)
}

fn error_fields(response: &Resp) -> Vec<String> {
    response
        .problem(422)
        .errors
        .into_iter()
        .map(|error| error.field)
        .collect()
}

fn detail(response: &Resp, status: u16) -> String {
    response.problem(status).detail.unwrap_or_default()
}

async fn outbox(pool: &PgPool, kind: &str) -> Vec<Value> {
    sqlx::query_scalar!(
        "SELECT payload FROM events.outbox WHERE type = $1 ORDER BY occurred_at, id",
        kind
    )
    .fetch_all(pool)
    .await
    .expect("outbox")
}

// =============================================================================================
// Drafts
// =============================================================================================

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_draft_is_saved_read_back_and_replaced_by_the_last_write(pool: PgPool) {
    let w = world(pool).await;
    let id = assignment(&w).await;

    let nothing = mine(&w, &w.student.token, id).await;
    assert_eq!(nothing["draft"], Value::Null);
    assert_eq!(nothing["attempts"], json!([]));
    assert_eq!(nothing["next_cursor"], Value::Null);

    let saved = json_of(save(&w, &w.student.token, id, essay("first")).await, 200);
    assert_eq!(saved["content"], essay("first"));
    assert!(saved["updated_at"].is_string());
    assert_eq!(
        mine(&w, &w.student.token, id).await["draft"]["content"],
        essay("first")
    );

    json_of(save(&w, &w.student.token, id, essay("second")).await, 200);
    let after = mine(&w, &w.student.token, id).await;
    assert_eq!(
        after["draft"]["content"],
        essay("second"),
        "the last write wins"
    );

    let rows = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM assessment.submissions WHERE assignment_id = $1"#,
        id
    )
    .fetch_one(&w.pool)
    .await
    .unwrap();
    assert_eq!(rows, 1, "one draft row, no versions");
    assert!(outbox(&w.pool, "submission.submitted").await.is_empty());
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn an_empty_document_can_be_saved_but_not_submitted(pool: PgPool) {
    let w = world(pool).await;
    let id = assignment(&w).await;
    for blank in [doc(json!([])), essay(""), essay("  \n ")] {
        json_of(save(&w, &w.student.token, id, blank).await, 200);
        let response = submit(&w, &w.student.token, id).await;
        assert_eq!(error_fields(&response), ["content"]);
    }
    // The refused submit left the draft in place and made no attempt.
    let after = mine(&w, &w.student.token, id).await;
    assert!(after["draft"].is_object());
    assert_eq!(after["attempts"], json!([]));
    assert!(outbox(&w.pool, "submission.submitted").await.is_empty());
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn the_student_profile_rejects_what_it_must_and_names_the_path(pool: PgPool) {
    let w = world(pool).await;
    let id = assignment(&w).await;
    let video = json!({"id": "v", "type": "video",
                       "props": {"provider": "youtube", "video_id": "dQw4w9WgXcQ"}});
    let bad_level = json!({"id": "h", "type": "heading", "props": {"level": 9}});
    let unknown = json!({"id": "u", "type": "blink"});
    let cases = [
        (
            "a video block",
            doc(json!([paragraph("a", "x"), video])),
            "content.blocks[1]",
        ),
        (
            "a bad heading level",
            doc(json!([paragraph("a", "x"), paragraph("b", "y"), bad_level])),
            "content.blocks[2].props.level",
        ),
        (
            "an unknown block",
            doc(json!([unknown])),
            "content.blocks[0].type",
        ),
        (
            "a document of the wrong version",
            json!({"v": 2, "blocks": []}),
            "content.v",
        ),
    ];
    for (what, content, field) in cases {
        let response = save(&w, &w.student.token, id, content).await;
        assert_eq!(error_fields(&response)[0], field, "{what}");
    }
    // A content that is not an object at all is a 422 on the field too, and nothing was stored.
    let response = call(
        &w,
        "PUT",
        &w.student.token,
        &draft_path(id),
        Some(json!({"content": "text"})),
    )
    .await;
    assert_eq!(error_fields(&response)[0], "content");
    assert_eq!(mine(&w, &w.student.token, id).await["draft"], Value::Null);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn an_assignment_narrows_the_student_editor_and_the_institution_narrows_it_first(
    pool: PgPool,
) {
    let mut settings = common::test_settings();
    let mut editor = EditorSettings::default();
    editor.profiles.insert(
        "student".to_owned(),
        ProfileSettings {
            features: ["headings", "text_styles", "lists", "code"]
                .map(str::to_owned)
                .into(),
        },
    );
    settings.editor = editor;
    let w = world_from(pool, settings).await;

    let heading = json!({"id": "h", "type": "heading", "props": {"level": 1},
                         "content": [{"type": "text", "text": "T"}]});
    let code = json!({"id": "c", "type": "code", "props": {"language": "rust", "source": "x"}});
    let list = json!({"id": "l", "type": "bulleted_list_item",
                      "content": [{"type": "text", "text": "a"}]});
    let quote = json!({"id": "q", "type": "quote", "content": [{"type": "text", "text": "q"}]});

    // No narrowing by the assignment: what the institution leaves.
    let plain = assignment(&w).await;
    json_of(
        save(
            &w,
            &w.student.token,
            plain,
            doc(json!([heading, code, list])),
        )
        .await,
        200,
    );
    let response = save(&w, &w.student.token, plain, doc(json!([quote]))).await;
    assert_eq!(
        error_fields(&response),
        ["content.blocks[0]"],
        "the institution's cut"
    );

    // The assignment cuts further: headings and code only.
    let narrow = assignment_with(
        &w,
        json!({}),
        json!({"editor": {"features": ["headings", "code"]}}),
    )
    .await;
    json_of(
        save(&w, &w.student.token, narrow, doc(json!([heading, code]))).await,
        200,
    );
    let response = save(&w, &w.student.token, narrow, doc(json!([heading, list]))).await;
    assert_eq!(error_fields(&response), ["content.blocks[1]"]);

    // It cannot widen: a feature the institution removed is refused when the config is written.
    let response = call(
        &w,
        "POST",
        &w.owner.token,
        &format!("/modules/{}/items", w.module),
        Some(json!({"kind": "assignment", "title": "E", "assignment": {
            "type": "text", "max_points": "5",
            "config": {"editor": {"features": ["headings", "table"]}}}})),
    )
    .await;
    assert_eq!(error_fields(&response), ["assignment.config"]);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_draft_can_be_discarded_also_when_there_is_none(pool: PgPool) {
    let w = world(pool).await;
    let id = assignment(&w).await;
    let gone = || async {
        call(&w, "DELETE", &w.student.token, &draft_path(id), None)
            .await
            .status
    };
    assert_eq!(gone().await, 204, "nothing to discard is not an error");
    json_of(save(&w, &w.student.token, id, essay("x")).await, 200);
    assert_eq!(gone().await, 204);
    assert_eq!(mine(&w, &w.student.token, id).await["draft"], Value::Null);
    // A draft after a discard starts again; another student's draft is untouched.
    json_of(save(&w, &w.other.token, id, essay("theirs")).await, 200);
    assert_eq!(gone().await, 204);
    assert!(mine(&w, &w.other.token, id).await["draft"].is_object());
    assert!(outbox(&w.pool, "submission.submitted").await.is_empty());
}

// =============================================================================================
// Attempts
// =============================================================================================

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_submit_consumes_the_draft_and_attempts_count_from_one(pool: PgPool) {
    let w = world(pool).await;
    let id = assignment(&w).await;
    for (number, text) in [(1, "one"), (2, "two"), (3, "three")] {
        let attempt = turn_in(&w, &w.student.token, id, text).await;
        assert_eq!(attempt["attempt"], number);
        assert_eq!(attempt["content"], essay(text));
        assert_eq!(attempt["is_late"], false);
        assert!(attempt["submitted_at"].is_string());
        let now = mine(&w, &w.student.token, id).await;
        assert_eq!(now["draft"], Value::Null, "the submit consumed the draft");
        assert_eq!(now["attempts"][0]["attempt"], number, "newest first");
    }
    let all = mine(&w, &w.student.token, id).await;
    let numbers: Vec<_> = all["attempts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|attempt| attempt["attempt"].as_i64().unwrap())
        .collect();
    assert_eq!(numbers, [3, 2, 1]);
    // Another student's numbering is their own.
    assert_eq!(turn_in(&w, &w.other.token, id, "mine").await["attempt"], 1);

    // The attempts are pages: newest first, then the older ones.
    let first = json_of(
        call(
            &w,
            "GET",
            &w.student.token,
            &format!("{}?limit=2", submission_path(id)),
            None,
        )
        .await,
        200,
    );
    assert_eq!(first["attempts"].as_array().unwrap().len(), 2);
    let cursor = first["next_cursor"].as_str().unwrap().to_owned();
    let rest = json_of(
        call(
            &w,
            "GET",
            &w.student.token,
            &format!("{}?limit=2&cursor={cursor}", submission_path(id)),
            None,
        )
        .await,
        200,
    );
    assert_eq!(rest["attempts"][0]["attempt"], 1);
    assert_eq!(rest["next_cursor"], Value::Null);

    // The rows: the draft became the attempt, three submitted rows and no draft.
    let (drafts, submitted) = sqlx::query!(
        r#"SELECT count(*) FILTER (WHERE status = 'draft') AS "drafts!",
                  count(*) FILTER (WHERE status = 'submitted') AS "submitted!"
           FROM assessment.submissions WHERE student_id = $1"#,
        w.student.id
    )
    .fetch_one(&w.pool)
    .await
    .map(|row| (row.drafts, row.submitted))
    .unwrap();
    assert_eq!((drafts, submitted), (0, 3));
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_second_submit_without_a_new_draft_is_a_conflict(pool: PgPool) {
    let w = world(pool).await;
    let id = assignment(&w).await;
    let response = submit(&w, &w.student.token, id).await;
    assert_eq!(
        detail(&response, 409),
        "there is no draft to submit",
        "never saved"
    );
    json_of(save(&w, &w.student.token, id, essay("x")).await, 200);
    json_of(submit(&w, &w.student.token, id).await, 201);
    let response = submit(&w, &w.student.token, id).await;
    assert_eq!(detail(&response, 409), "there is no draft to submit");
    assert_eq!(
        mine(&w, &w.student.token, id).await["attempts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(outbox(&w.pool, "submission.submitted").await.len(), 1);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn two_submits_sent_together_make_one_attempt(pool: PgPool) {
    let w = world(pool).await;
    let id = assignment(&w).await;
    json_of(save(&w, &w.student.token, id, essay("x")).await, 200);
    let (a, b) = tokio::join!(
        submit(&w, &w.student.token, id),
        submit(&w, &w.student.token, id)
    );
    let mut statuses = [a.status, b.status];
    statuses.sort_unstable();
    assert_eq!(statuses, [201, 409]);
    assert_eq!(
        mine(&w, &w.student.token, id).await["attempts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(outbox(&w.pool, "submission.submitted").await.len(), 1);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn max_attempts_limits_submits_and_a_lowered_limit_only_blocks_further_ones(pool: PgPool) {
    let w = world(pool).await;
    let id = assignment_with(&w, json!({"max_attempts": 2}), json!({})).await;
    turn_in(&w, &w.student.token, id, "one").await;
    turn_in(&w, &w.student.token, id, "two").await;
    json_of(save(&w, &w.student.token, id, essay("three")).await, 200);
    let response = submit(&w, &w.student.token, id).await;
    assert_eq!(detail(&response, 409), "the attempt limit is reached");
    assert!(
        mine(&w, &w.student.token, id).await["draft"].is_object(),
        "the draft stays"
    );
    // Another student has their own two.
    turn_in(&w, &w.other.token, id, "one").await;

    // Raising the limit lets the waiting draft through.
    let patch = async |attempts: Value| {
        call(
            &w,
            "PATCH",
            &w.owner.token,
            &format!("/items/{id}"),
            Some(json!({"max_attempts": attempts})),
        )
        .await
    };
    json_of(patch(json!(3)).await, 200);
    assert_eq!(
        json_of(submit(&w, &w.student.token, id).await, 201)["attempt"],
        3
    );
    // Lowering it below what the student has leaves their attempts alone and blocks the next.
    json_of(patch(json!(1)).await, 200);
    assert_eq!(
        mine(&w, &w.student.token, id).await["attempts"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    json_of(save(&w, &w.student.token, id, essay("four")).await, 200);
    assert_eq!(
        detail(&submit(&w, &w.student.token, id).await, 409),
        "the attempt limit is reached"
    );
    // No limit at all.
    json_of(patch(Value::Null).await, 200);
    assert_eq!(
        json_of(submit(&w, &w.student.token, id).await, 201)["attempt"],
        4
    );
}

// =============================================================================================
// The window
// =============================================================================================

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn before_the_deadline_work_is_on_time(pool: PgPool) {
    let w = world(pool).await;
    let id = assignment_with(
        &w,
        deadline(Utc::now() + Duration::days(1), false, None),
        json!({}),
    )
    .await;
    let attempt = turn_in(&w, &w.student.token, id, "x").await;
    assert_eq!(attempt["is_late"], false);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn after_the_deadline_without_late_work_a_save_and_a_submit_are_refused(pool: PgPool) {
    let w = world(pool).await;
    let id = assignment_with(
        &w,
        deadline(Utc::now() - Duration::hours(1), false, None),
        json!({}),
    )
    .await;
    let response = save(&w, &w.student.token, id, essay("x")).await;
    assert_eq!(detail(&response, 409), "the submission window is closed");
    let response = submit(&w, &w.student.token, id).await;
    assert_eq!(detail(&response, 409), "the submission window is closed");
    assert!(outbox(&w.pool, "submission.submitted").await.is_empty());
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn late_work_is_taken_and_marked_until_late_until(pool: PgPool) {
    let w = world(pool).await;
    // Late work without an end.
    let open_ended = assignment_with(
        &w,
        deadline(Utc::now() - Duration::days(30), true, None),
        json!({}),
    )
    .await;
    let attempt = turn_in(&w, &w.student.token, open_ended, "x").await;
    assert_eq!(attempt["is_late"], true);
    // Late work inside the window.
    let now = Utc::now();
    let inside = assignment_with(
        &w,
        deadline(
            now - Duration::hours(2),
            true,
            Some(now + Duration::hours(2)),
        ),
        json!({}),
    )
    .await;
    assert_eq!(
        turn_in(&w, &w.student.token, inside, "x").await["is_late"],
        true
    );
    // After late_until the window is closed for a save and a submit.
    let over = assignment_with(
        &w,
        deadline(
            now - Duration::hours(3),
            true,
            Some(now - Duration::hours(1)),
        ),
        json!({}),
    )
    .await;
    assert_eq!(
        save(&w, &w.student.token, over, essay("x"))
            .await
            .problem(409)
            .status,
        409
    );
    assert_eq!(
        submit(&w, &w.student.token, over).await.problem(409).status,
        409
    );
    // The late flag is stored with the attempt and shown to staff.
    let listed = json_of(
        call(
            &w,
            "GET",
            &w.owner.token,
            &format!("/assignments/{inside}/submissions"),
            None,
        )
        .await,
        200,
    );
    let row = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["student_id"] == json!(w.student.id))
        .unwrap();
    assert_eq!(row["latest"]["is_late"], true);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_closed_window_keeps_the_draft_and_lets_it_be_discarded(pool: PgPool) {
    let w = world(pool).await;
    let id = assignment_with(
        &w,
        deadline(Utc::now() + Duration::days(1), false, None),
        json!({}),
    )
    .await;
    json_of(save(&w, &w.student.token, id, essay("kept")).await, 200);
    // The teacher moves the deadline into the past: it applies from the student's next action.
    json_of(
        call(
            &w,
            "PATCH",
            &w.owner.token,
            &format!("/items/{id}"),
            Some(json!({"due_at": stamp(Utc::now() - Duration::minutes(1))})),
        )
        .await,
        200,
    );
    assert_eq!(
        submit(&w, &w.student.token, id).await.problem(409).status,
        409
    );
    assert_eq!(
        save(&w, &w.student.token, id, essay("more"))
            .await
            .problem(409)
            .status,
        409
    );
    assert_eq!(
        mine(&w, &w.student.token, id).await["draft"]["content"],
        essay("kept")
    );
    assert_eq!(
        call(&w, "DELETE", &w.student.token, &draft_path(id), None)
            .await
            .status,
        204
    );
    // Reopening the window lets the student work again; earlier attempts were never touched.
    json_of(
        call(
            &w,
            "PATCH",
            &w.owner.token,
            &format!("/items/{id}"),
            Some(json!({"due_at": stamp(Utc::now() + Duration::days(1))})),
        )
        .await,
        200,
    );
    assert_eq!(
        turn_in(&w, &w.student.token, id, "again").await["attempt"],
        1
    );
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn changing_the_settings_does_not_touch_the_attempts_already_made(pool: PgPool) {
    let w = world(pool).await;
    let id = assignment_with(
        &w,
        deadline(Utc::now() + Duration::days(1), false, None),
        json!({}),
    )
    .await;
    let first = turn_in(&w, &w.student.token, id, "on time").await;
    json_of(
        call(
            &w,
            "PATCH",
            &w.owner.token,
            &format!("/items/{id}"),
            Some(json!({"due_at": stamp(Utc::now() - Duration::days(1)), "max_attempts": 1})),
        )
        .await,
        200,
    );
    let kept = mine(&w, &w.student.token, id).await;
    assert_eq!(kept["attempts"][0], first, "the attempt is as it was");
}

// =============================================================================================
// Types that take no submissions
// =============================================================================================

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn an_assignment_that_takes_no_submissions_has_none(pool: PgPool) {
    let w = world(pool).await;
    let oral = id_of(&json_of(
        call(
            &w,
            "POST",
            &w.owner.token,
            &format!("/modules/{}/items", w.module),
            Some(json!({"kind": "assignment", "title": "Oral",
                        "assignment": {"type": "no_submission", "max_points": "5"}})),
        )
        .await,
        201,
    ));
    json_of(
        call(
            &w,
            "POST",
            &w.owner.token,
            &format!("/items/{oral}/publish"),
            None,
        )
        .await,
        200,
    );
    for (method, path, token, body) in [
        (
            "PUT",
            draft_path(oral),
            &w.student.token,
            Some(json!({"content": essay("x")})),
        ),
        ("DELETE", draft_path(oral), &w.student.token, None),
        ("GET", submission_path(oral), &w.student.token, None),
        (
            "POST",
            format!("{}/submit", submission_path(oral)),
            &w.student.token,
            None,
        ),
        (
            "GET",
            format!("/assignments/{oral}/submissions"),
            &w.owner.token,
            None,
        ),
        (
            "GET",
            format!("/assignments/{oral}/submissions/{}", w.student.id),
            &w.owner.token,
            None,
        ),
    ] {
        let response = call(&w, method, token, &path, body).await;
        assert_eq!(
            detail(&response, 409),
            "this assignment takes no submissions",
            "{method} {path}"
        );
    }
}

// =============================================================================================
// Who may do and read what
// =============================================================================================

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn only_a_student_of_the_course_who_sees_the_assignment_can_save_and_submit(pool: PgPool) {
    let w = world(pool).await;
    let id = assignment(&w).await;
    let body = || Some(json!({"content": essay("x")}));
    let submit_path = format!("{}/submit", submission_path(id));

    // Visible, but not theirs to do: 403 for staff and the admin.
    for (who, token) in [
        ("owner", &w.owner.token),
        ("assistant", &w.assistant.token),
        ("admin", &w.admin.token),
    ] {
        assert_eq!(
            call(&w, "PUT", token, &draft_path(id), body())
                .await
                .problem(403)
                .status,
            403,
            "{who}"
        );
        assert_eq!(
            call(&w, "DELETE", token, &draft_path(id), None)
                .await
                .problem(403)
                .status,
            403,
            "{who}"
        );
        assert_eq!(
            call(&w, "POST", token, &submit_path, None)
                .await
                .problem(403)
                .status,
            403,
            "{who}"
        );
        assert_eq!(
            call(&w, "GET", token, &submission_path(id), None)
                .await
                .problem(403)
                .status,
            403,
            "{who}"
        );
    }
    // Not visible at all: 404 for a teacher outside the course.
    for (method, path, request_body) in [
        ("PUT", draft_path(id), body()),
        ("DELETE", draft_path(id), None),
        ("POST", submit_path.clone(), None),
        ("GET", submission_path(id), None),
    ] {
        let response = call(&w, method, &w.stranger.token, &path, request_body).await;
        assert_eq!(response.problem(404).status, 404, "{method} {path}");
    }
    // Not logged in.
    let anonymous = w
        .http
        .put(&draft_path(id))
        .json(&json!({"content": essay("x")}))
        .send()
        .await;
    assert_eq!(anonymous.problem(401).status, 401);
    assert_eq!(
        w.http.post(&submit_path).send().await.problem(401).status,
        401
    );
    // Nothing was written by any of them.
    let rows = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM assessment.submissions"#)
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_student_of_another_course_and_an_unknown_assignment_give_404(pool: PgPool) {
    let w = world(pool).await;
    let id = assignment(&w).await;
    let elsewhere = user(&w.pool, &w.http, "elsewhere@x.edu", Role::Student).await;
    let other_course = create_course(&w.http, &w.owner.token, "Geometry").await;
    put_member(
        &w.http,
        &w.owner.token,
        other_course,
        elsewhere.id,
        "student",
    )
    .await;
    publish_course(&w.http, &w.owner.token, other_course).await;

    assert_eq!(
        save(&w, &elsewhere.token, id, essay("x"))
            .await
            .problem(404)
            .status,
        404
    );
    assert_eq!(
        submit(&w, &elsewhere.token, id).await.problem(404).status,
        404
    );
    let unknown = Uuid::now_v7();
    assert_eq!(
        save(&w, &w.student.token, unknown, essay("x"))
            .await
            .problem(404)
            .status,
        404
    );
    // Staff routes answer the same for a course the caller cannot see.
    let response = call(
        &w,
        "GET",
        &elsewhere.token,
        &format!("/assignments/{id}/submissions"),
        None,
    )
    .await;
    assert_eq!(response.problem(404).status, 404);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_student_reaches_an_assignment_only_once_it_is_released(pool: PgPool) {
    let w = world(pool).await;
    // A draft item in a published module.
    let hidden = id_of(&json_of(
        call(
            &w,
            "POST",
            &w.owner.token,
            &format!("/modules/{}/items", w.module),
            Some(json!({"kind": "assignment", "title": "Hidden",
                        "assignment": {"type": "text", "max_points": "5"}})),
        )
        .await,
        201,
    ));
    assert_eq!(
        save(&w, &w.student.token, hidden, essay("x"))
            .await
            .problem(404)
            .status,
        404
    );
    assert_eq!(
        submit(&w, &w.student.token, hidden)
            .await
            .problem(404)
            .status,
        404
    );
    // Scheduled for later.
    json_of(
        call(
            &w,
            "POST",
            &w.owner.token,
            &format!("/items/{hidden}/publish"),
            None,
        )
        .await,
        200,
    );
    json_of(
        call(
            &w,
            "PATCH",
            &w.owner.token,
            &format!("/items/{hidden}"),
            Some(json!({"publish_at": stamp(Utc::now() + Duration::days(1))})),
        )
        .await,
        200,
    );
    assert_eq!(
        save(&w, &w.student.token, hidden, essay("x"))
            .await
            .problem(404)
            .status,
        404
    );
    // Staff see it and are told they may not submit.
    assert_eq!(
        save(&w, &w.owner.token, hidden, essay("x"))
            .await
            .problem(403)
            .status,
        403
    );
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_student_who_was_excluded_or_left_the_group_can_no_longer_work(pool: PgPool) {
    let w = world(pool).await;
    let id = assignment(&w).await;
    // A student who is on the course only through a linked group.
    let grouped = user(&w.pool, &w.http, "grouped@x.edu", Role::Student).await;
    let group = id_of(&json_of(
        call(
            &w,
            "POST",
            &w.admin.token,
            "/groups",
            Some(json!({"name": "7-A"})),
        )
        .await,
        201,
    ));
    json_of(
        call(
            &w,
            "POST",
            &w.admin.token,
            &format!("/groups/{group}/members"),
            Some(json!({"user_ids": [grouped.id]})),
        )
        .await,
        200,
    );
    json_of(
        call(
            &w,
            "POST",
            &w.owner.token,
            &format!("/courses/{}/groups", w.course),
            Some(json!({"group_id": group})),
        )
        .await,
        201,
    );
    turn_in(&w, &grouped.token, id, "from the group").await;

    // Excluded from this course only: the group is untouched.
    let removed = call(
        &w,
        "DELETE",
        &w.owner.token,
        &format!("/courses/{}/members/{}", w.course, grouped.id),
        None,
    )
    .await;
    assert_eq!(removed.status, 204);
    assert_eq!(
        save(&w, &grouped.token, id, essay("x"))
            .await
            .problem(404)
            .status,
        404
    );
    assert_eq!(
        submit(&w, &grouped.token, id).await.problem(404).status,
        404
    );
    // Their attempt stays and staff can still open it (an exclusion never touches submissions).
    let attempts = json_of(
        call(
            &w,
            "GET",
            &w.owner.token,
            &format!("/assignments/{id}/submissions/{}", grouped.id),
            None,
        )
        .await,
        200,
    );
    assert_eq!(attempts["items"].as_array().unwrap().len(), 1);
    // They are not on the staff list any more: it lists the roster.
    let listed = json_of(
        call(
            &w,
            "GET",
            &w.owner.token,
            &format!("/assignments/{id}/submissions"),
            None,
        )
        .await,
        200,
    );
    assert!(
        listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["student_id"] != json!(grouped.id))
    );

    // Another student leaves the group: restored first, then removed from the group.
    let leaver = user(&w.pool, &w.http, "leaver@x.edu", Role::Student).await;
    json_of(
        call(
            &w,
            "POST",
            &w.admin.token,
            &format!("/groups/{group}/members"),
            Some(json!({"user_ids": [leaver.id]})),
        )
        .await,
        200,
    );
    json_of(save(&w, &leaver.token, id, essay("x")).await, 200);
    assert_eq!(
        call(
            &w,
            "DELETE",
            &w.admin.token,
            &format!("/groups/{group}/members/{}", leaver.id),
            None,
        )
        .await
        .status,
        204
    );
    assert_eq!(
        save(&w, &leaver.token, id, essay("y"))
            .await
            .problem(404)
            .status,
        404
    );
    assert_eq!(submit(&w, &leaver.token, id).await.problem(404).status, 404);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_student_reads_only_their_own_work_and_never_staff_routes(pool: PgPool) {
    let w = world(pool).await;
    let id = assignment(&w).await;
    turn_in(&w, &w.student.token, id, "secret one").await;
    json_of(
        save(&w, &w.student.token, id, essay("secret draft")).await,
        200,
    );

    let theirs = mine(&w, &w.other.token, id).await;
    assert_eq!(theirs["draft"], Value::Null);
    assert_eq!(theirs["attempts"], json!([]));
    let raw = theirs.to_string();
    assert!(!raw.contains("secret"), "{raw}");

    for path in [
        format!("/assignments/{id}/submissions"),
        format!("/assignments/{id}/submissions/{}", w.student.id),
    ] {
        let response = call(&w, "GET", &w.other.token, &path, None).await;
        assert_eq!(response.problem(403).status, 403, "{path}");
    }
}

// =============================================================================================
// What staff read
// =============================================================================================

/// Gives the students distinct surnames so that the roster order is not the order of ids.
async fn name_students(w: &World) {
    for (user, name, surname) in [
        (&w.student, "Anna", "Zielinska"),
        (&w.other, "Olek", "Adamski"),
    ] {
        sqlx::query!(
            "UPDATE identity.users SET name = $2, surname = $3 WHERE id = $1",
            user.id,
            name,
            surname
        )
        .execute(&w.pool)
        .await
        .unwrap();
    }
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn staff_list_every_student_in_roster_order_with_the_latest_attempt(pool: PgPool) {
    let w = world(pool).await;
    name_students(&w).await;
    let id = assignment(&w).await;
    turn_in(&w, &w.student.token, id, "one").await;
    turn_in(&w, &w.student.token, id, "two").await;

    for token in [&w.owner.token, &w.assistant.token, &w.admin.token] {
        let listed = json_of(
            call(
                &w,
                "GET",
                token,
                &format!("/assignments/{id}/submissions"),
                None,
            )
            .await,
            200,
        );
        let rows = listed["items"].as_array().unwrap();
        assert_eq!(rows.len(), 2, "the two students of the roster, no staff");
        assert_eq!(rows[0]["surname"], "Adamski", "surname order");
        assert_eq!(rows[0]["status"], "not_submitted");
        assert_eq!(rows[0]["latest"], Value::Null);
        assert_eq!(rows[1]["student_id"], json!(w.student.id));
        assert_eq!(rows[1]["status"], "submitted");
        assert_eq!(rows[1]["latest"]["attempt"], 2, "the latest attempt");
        assert_eq!(rows[1]["latest"]["is_late"], false);
        assert!(
            rows[1]["latest"].get("content").is_none(),
            "no content in the list"
        );
        assert_eq!(listed["next_cursor"], Value::Null);
    }

    // Pages follow the roster.
    let first = json_of(
        call(
            &w,
            "GET",
            &w.owner.token,
            &format!("/assignments/{id}/submissions?limit=1"),
            None,
        )
        .await,
        200,
    );
    assert_eq!(first["items"][0]["surname"], "Adamski");
    let cursor = first["next_cursor"].as_str().unwrap();
    let second = json_of(
        call(
            &w,
            "GET",
            &w.owner.token,
            &format!("/assignments/{id}/submissions?limit=1&cursor={cursor}"),
            None,
        )
        .await,
        200,
    );
    assert_eq!(second["items"][0]["surname"], "Zielinska");
    assert_eq!(second["next_cursor"], Value::Null);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn staff_open_all_attempts_of_a_student_newest_first(pool: PgPool) {
    let w = world(pool).await;
    let id = assignment(&w).await;
    for text in ["one", "two", "three"] {
        turn_in(&w, &w.student.token, id, text).await;
    }
    let path = format!("/assignments/{id}/submissions/{}", w.student.id);
    let page = json_of(call(&w, "GET", &w.assistant.token, &path, None).await, 200);
    let numbers: Vec<_> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|attempt| attempt["attempt"].as_i64().unwrap())
        .collect();
    assert_eq!(numbers, [3, 2, 1]);
    assert_eq!(
        page["items"][2]["content"],
        essay("one"),
        "earlier attempts stay visible"
    );

    let first = json_of(
        call(&w, "GET", &w.owner.token, &format!("{path}?limit=2"), None).await,
        200,
    );
    let cursor = first["next_cursor"].as_str().unwrap();
    let rest = json_of(
        call(
            &w,
            "GET",
            &w.owner.token,
            &format!("{path}?limit=2&cursor={cursor}"),
            None,
        )
        .await,
        200,
    );
    assert_eq!(rest["items"].as_array().unwrap().len(), 1);
    assert_eq!(rest["next_cursor"], Value::Null);

    // A user who is neither on the roster nor the author of an attempt is not found; so is a
    // teacher's id.
    for unknown in [Uuid::now_v7(), w.owner.id] {
        let response = call(
            &w,
            "GET",
            &w.owner.token,
            &format!("/assignments/{id}/submissions/{unknown}"),
            None,
        )
        .await;
        assert_eq!(response.problem(404).status, 404);
    }
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn staff_never_see_a_draft(pool: PgPool) {
    let w = world(pool).await;
    let id = assignment(&w).await;
    // A student with only a draft, and one with an attempt and a newer draft.
    json_of(
        save(&w, &w.student.token, id, essay("DRAFT-ONLY-TEXT")).await,
        200,
    );
    turn_in(&w, &w.other.token, id, "submitted text").await;
    json_of(
        save(&w, &w.other.token, id, essay("NEWER-DRAFT-TEXT")).await,
        200,
    );

    for token in [&w.owner.token, &w.assistant.token, &w.admin.token] {
        let list = call(
            &w,
            "GET",
            token,
            &format!("/assignments/{id}/submissions"),
            None,
        )
        .await;
        let raw = String::from_utf8_lossy(&list.body).into_owned();
        let list = list.json();
        assert!(!raw.contains("DRAFT"), "{raw}");
        let student_row = list["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["student_id"] == json!(w.student.id))
            .unwrap();
        assert_eq!(student_row["status"], "not_submitted");
        assert_eq!(student_row["latest"], Value::Null);

        for (student, expected) in [(w.student.id, 0), (w.other.id, 1)] {
            let page = call(
                &w,
                "GET",
                token,
                &format!("/assignments/{id}/submissions/{student}"),
                None,
            )
            .await;
            let raw = String::from_utf8_lossy(&page.body).into_owned();
            assert!(!raw.contains("DRAFT"), "{raw}");
            assert_eq!(page.json()["items"].as_array().unwrap().len(), expected);
        }
    }
}

// =============================================================================================
// Events
// =============================================================================================

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_submit_emits_one_event_with_ids_only_and_a_draft_emits_none(pool: PgPool) {
    let w = world(pool).await;
    let id = assignment_with(&w, json!({"max_attempts": 1}), json!({})).await;
    json_of(save(&w, &w.student.token, id, essay("x")).await, 200);
    json_of(save(&w, &w.student.token, id, essay("y")).await, 200);
    call(&w, "DELETE", &w.student.token, &draft_path(id), None).await;
    json_of(save(&w, &w.student.token, id, essay("z")).await, 200);
    assert!(outbox(&w.pool, "submission.submitted").await.is_empty());

    let attempt = json_of(submit(&w, &w.student.token, id).await, 201);
    let events = outbox(&w.pool, "submission.submitted").await;
    assert_eq!(
        events,
        [json!({
            "assignment_id": id,
            "student_id": w.student.id,
            "course_id": w.course,
            "submission_id": attempt["id"],
        })]
    );
    // Refused submits add none.
    json_of(save(&w, &w.student.token, id, essay("again")).await, 200);
    assert_eq!(
        submit(&w, &w.student.token, id).await.problem(409).status,
        409
    );
    assert_eq!(
        submit(&w, &w.student.token, id).await.problem(409).status,
        409
    );
    assert_eq!(outbox(&w.pool, "submission.submitted").await.len(), 1);
    let total = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM events.outbox"#)
        .fetch_one(&w.pool)
        .await
        .unwrap();
    assert_eq!(total, 1, "no other event was written");
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn the_submission_endpoints_are_in_the_openapi_document(pool: PgPool) {
    let w = world(pool).await;
    let spec = w.http.get("/openapi.json").send().await.json();
    let paths = spec["paths"].as_object().unwrap();
    for (path, methods) in [
        (
            "/api/v1/assignments/{id}/submission/draft",
            vec!["put", "delete"],
        ),
        ("/api/v1/assignments/{id}/submission", vec!["get"]),
        ("/api/v1/assignments/{id}/submission/submit", vec!["post"]),
        ("/api/v1/assignments/{id}/submissions", vec!["get"]),
        (
            "/api/v1/assignments/{id}/submissions/{student_id}",
            vec!["get"],
        ),
    ] {
        let item = paths
            .get(path)
            .unwrap_or_else(|| panic!("{path} is not documented"));
        for method in methods {
            assert!(item.get(method).is_some(), "{method} {path}");
        }
    }
}

// =============================================================================================
// The table, against a writer that skips the service
// =============================================================================================

/// Inserts a row and returns the name of the constraint that refused it, `None` when it went in.
async fn insert(
    pool: &PgPool,
    assignment: Uuid,
    student: Uuid,
    status: &str,
    attempt: Option<i32>,
    submitted: bool,
    is_late: bool,
) -> Option<String> {
    let submitted_at = submitted.then(Utc::now);
    let result = sqlx::query!(
        "INSERT INTO assessment.submissions
             (id, assignment_id, student_id, status, attempt, content, submitted_at, is_late)
         VALUES ($1, $2, $3, $4, $5, '{}', $6, $7)",
        Uuid::now_v7(),
        assignment,
        student,
        status,
        attempt,
        submitted_at,
        is_late
    )
    .execute(pool)
    .await;
    match result {
        Ok(_) => None,
        Err(sqlx::Error::Database(error)) => Some(error.constraint().unwrap_or("?").to_owned()),
        Err(other) => panic!("unexpected error: {other}"),
    }
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn the_table_refuses_what_the_rules_forbid(pool: PgPool) {
    common::seed_user(&pool, "s@x.edu", Role::Student).await;
    let student = sqlx::query_scalar!("SELECT id FROM identity.users")
        .fetch_one(&pool)
        .await
        .unwrap();
    let a = Uuid::now_v7();

    // A draft has no attempt number and no submit time; a submitted row has both.
    for (what, status, attempt, submitted) in [
        ("a draft with an attempt", "draft", Some(1), false),
        ("a draft with a submit time", "draft", None, true),
        (
            "a submitted row without an attempt",
            "submitted",
            None,
            true,
        ),
        (
            "a submitted row without a submit time",
            "submitted",
            Some(1),
            false,
        ),
        ("attempt 0", "submitted", Some(0), true),
    ] {
        let refused = insert(&pool, a, student, status, attempt, submitted, false).await;
        assert!(refused.is_some(), "{what}");
    }
    assert_eq!(
        insert(&pool, a, student, "waiting", None, false, false)
            .await
            .as_deref(),
        Some("submissions_status_check")
    );
    // Only a submitted row can be late.
    assert!(
        insert(&pool, a, student, "draft", None, false, true)
            .await
            .is_some()
    );

    // One draft per (assignment, student); another student or assignment is a separate one.
    assert_eq!(
        insert(&pool, a, student, "draft", None, false, false).await,
        None
    );
    assert_eq!(
        insert(&pool, a, student, "draft", None, false, false)
            .await
            .as_deref(),
        Some("submissions_one_draft_idx")
    );
    assert_eq!(
        insert(&pool, Uuid::now_v7(), student, "draft", None, false, false).await,
        None
    );

    // An attempt number is used once per (assignment, student), however many drafts come after.
    assert_eq!(
        insert(&pool, a, student, "submitted", Some(1), true, false).await,
        None
    );
    assert_eq!(
        insert(&pool, a, student, "submitted", Some(1), true, false)
            .await
            .as_deref(),
        Some("submissions_attempt_idx")
    );
    assert_eq!(
        insert(&pool, a, student, "submitted", Some(2), true, true).await,
        None
    );
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_submitted_row_is_never_changed_but_a_draft_can_become_one(pool: PgPool) {
    common::seed_user(&pool, "s@x.edu", Role::Student).await;
    let student = sqlx::query_scalar!("SELECT id FROM identity.users")
        .fetch_one(&pool)
        .await
        .unwrap();
    let a = Uuid::now_v7();
    assert_eq!(
        insert(&pool, a, student, "draft", None, false, false).await,
        None
    );

    let turned = sqlx::query!(
        "UPDATE assessment.submissions
         SET status = 'submitted', attempt = 1, submitted_at = now() WHERE assignment_id = $1",
        a
    )
    .execute(&pool)
    .await;
    assert!(turned.is_ok(), "{turned:?}");

    for change in [
        sqlx::query!(
            "UPDATE assessment.submissions SET content = '{\"v\": 1}' WHERE assignment_id = $1",
            a
        ),
        sqlx::query!(
            "UPDATE assessment.submissions SET is_late = true WHERE assignment_id = $1",
            a
        ),
        sqlx::query!(
            "UPDATE assessment.submissions SET attempt = 2 WHERE assignment_id = $1",
            a
        ),
        sqlx::query!(
            "UPDATE assessment.submissions SET status = 'draft', attempt = NULL, submitted_at = NULL WHERE assignment_id = $1",
            a
        ),
    ] {
        let error = change
            .execute(&pool)
            .await
            .expect_err("a submitted row is final");
        assert!(error.to_string().contains("never changed"), "{error}");
    }
}
