mod common;

use common::{Http, Resp, create_course, id_of, publish_course, put_member, seed_and_login};
use serde_json::{Value, json};
use server::identity::Role;
use sqlx::PgPool;
use uuid::Uuid;

const FUTURE: &str = "2099-01-01T00:00:00Z";
const PAST: &str = "2000-01-01T00:00:00Z";

struct User {
    id: Uuid,
    token: String,
}

async fn user(pool: &PgPool, http: &Http, email: &str, role: Role) -> User {
    let (id, token) = seed_and_login(pool, http, email, role).await;
    User { id, token }
}

/// A published course with an owner, an assistant and a student, plus a teacher who is not a
/// member and an admin.
struct World {
    http: Http,
    pool: PgPool,
    owner: User,
    assistant: User,
    student: User,
    stranger: User,
    admin: User,
    course: Uuid,
}

async fn world(pool: PgPool) -> World {
    let http = Http::new(&common::spawn(pool.clone()).await);
    let owner = user(&pool, &http, "owner@x.edu", Role::Teacher).await;
    let assistant = user(&pool, &http, "assistant@x.edu", Role::Student).await;
    let student = user(&pool, &http, "student@x.edu", Role::Student).await;
    let stranger = user(&pool, &http, "stranger@x.edu", Role::Teacher).await;
    let admin = user(&pool, &http, "admin@x.edu", Role::Admin).await;
    let course = create_course(&http, &owner.token, "Algebra 7-A").await;
    put_member(&http, &owner.token, course, assistant.id, "assistant").await;
    put_member(&http, &owner.token, course, student.id, "student").await;
    publish_course(&http, &owner.token, course).await;
    World {
        http,
        pool,
        owner,
        assistant,
        student,
        stranger,
        admin,
        course,
    }
}

// --- request helpers ---------------------------------------------------------------------------

async fn get(http: &Http, token: &str, path: &str) -> Resp {
    http.get(path).bearer(token).send().await
}

async fn post(http: &Http, token: &str, path: &str, body: Value) -> Resp {
    http.post(path).bearer(token).json(&body).send().await
}

async fn patch(http: &Http, token: &str, path: &str, body: Value) -> Resp {
    http.patch(path).bearer(token).json(&body).send().await
}

async fn put(http: &Http, token: &str, path: &str, body: Value) -> Resp {
    http.put(path).bearer(token).json(&body).send().await
}

async fn delete(http: &Http, token: &str, path: &str) -> Resp {
    http.delete(path).bearer(token).send().await
}

fn created(response: &Resp) -> Value {
    assert_eq!(
        response.status,
        201,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    response.json()
}

fn ok(response: &Resp) -> Value {
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    response.json()
}

fn no_content(response: &Resp) {
    assert_eq!(
        response.status,
        204,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
}

async fn module(w: &World, title: &str, parent: Option<Uuid>) -> Uuid {
    module_in(w, w.course, title, parent).await
}

async fn module_in(w: &World, course: Uuid, title: &str, parent: Option<Uuid>) -> Uuid {
    let response = post(
        &w.http,
        &w.owner.token,
        &format!("/courses/{course}/modules"),
        json!({"title": title, "parent_id": parent}),
    )
    .await;
    id_of(&created(&response))
}

async fn module_as(w: &World, token: &str, title: &str, parent: Option<Uuid>) -> Uuid {
    let response = post(
        &w.http,
        token,
        &format!("/courses/{}/modules", w.course),
        json!({"title": title, "parent_id": parent}),
    )
    .await;
    id_of(&created(&response))
}

async fn item(w: &World, module: Uuid, title: &str) -> Uuid {
    let response = post(
        &w.http,
        &w.owner.token,
        &format!("/modules/{module}/items"),
        json!({"kind": "lesson", "title": title}),
    )
    .await;
    id_of(&created(&response))
}

async fn publish_module(w: &World, module: Uuid, cascade: bool) -> Value {
    let response = post(
        &w.http,
        &w.owner.token,
        &format!("/modules/{module}/publish"),
        json!({"cascade": cascade}),
    )
    .await;
    ok(&response)
}

async fn publish_item(w: &World, item: Uuid) -> Value {
    ok(&post(
        &w.http,
        &w.owner.token,
        &format!("/items/{item}/publish"),
        json!(null),
    )
    .await)
}

async fn schedule_module(w: &World, module: Uuid, at: &str) {
    ok(&patch(
        &w.http,
        &w.owner.token,
        &format!("/modules/{module}"),
        json!({"publish_at": at}),
    )
    .await);
}

async fn schedule_item(w: &World, item: Uuid, at: &str) {
    ok(&patch(
        &w.http,
        &w.owner.token,
        &format!("/items/{item}"),
        json!({"publish_at": at}),
    )
    .await);
}

async fn tree(w: &World, token: &str) -> Value {
    ok(&get(&w.http, token, &format!("/courses/{}/content", w.course)).await)
}

fn titles(list: &Value) -> Vec<&str> {
    list.as_array()
        .expect("array")
        .iter()
        .map(|entry| entry["title"].as_str().expect("title"))
        .collect()
}

fn top<'a>(tree: &'a Value, title: &str) -> &'a Value {
    tree["modules"]
        .as_array()
        .expect("modules")
        .iter()
        .find(|m| m["title"] == title)
        .unwrap_or_else(|| panic!("no top-level module {title}"))
}

fn first_error_field(response: &Resp) -> String {
    response.problem(422).errors[0].field.clone()
}

async fn audit_rows(pool: &PgPool) -> Vec<(String, String, Uuid, Value)> {
    sqlx::query!(
        "SELECT action, resource_type, resource_id, meta FROM events.audit_log ORDER BY at, id"
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|r| (r.action, r.resource_type, r.resource_id, r.meta))
    .collect()
}

// =============================================================================================
// Creating and reading the tree
// =============================================================================================

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn modules_sub_modules_and_lessons_form_an_ordered_tree(pool: PgPool) {
    let w = world(pool).await;
    let lectures = module(&w, "Lectures", None).await;
    let practice = module(&w, "Practice", None).await;
    let l1 = module(&w, "Lecture 1", Some(lectures)).await;
    let l2 = module(&w, "Lecture 2", Some(lectures)).await;
    let homework = item(&w, lectures, "Homework 1").await;
    let slides = item(&w, l1, "Slides").await;

    let tree = tree(&w, &w.owner.token).await;
    assert_eq!(titles(&tree["modules"]), ["Lectures", "Practice"]);
    let lectures_node = top(&tree, "Lectures");
    assert_eq!(lectures_node["id"], json!(lectures));
    assert_eq!(lectures_node["parent_id"], Value::Null);
    assert_eq!(lectures_node["status"], "draft");
    assert_eq!(
        titles(&lectures_node["submodules"]),
        ["Lecture 1", "Lecture 2"]
    );
    assert_eq!(lectures_node["submodules"][0]["id"], json!(l1));
    assert_eq!(lectures_node["submodules"][1]["id"], json!(l2));
    assert_eq!(lectures_node["submodules"][0]["parent_id"], json!(lectures));
    // Direct items and sub-module items are listed apart, as summaries without a body.
    assert_eq!(lectures_node["items"][0]["id"], json!(homework));
    assert_eq!(
        lectures_node["submodules"][0]["items"][0]["id"],
        json!(slides)
    );
    assert!(lectures_node["items"][0].get("body").is_none());
    assert_eq!(lectures_node["items"][0]["kind"], "lesson");
    assert_eq!(top(&tree, "Practice")["id"], json!(practice));
    assert_eq!(top(&tree, "Practice")["submodules"], json!([]));
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn new_rows_take_the_last_position_of_their_own_sequence(pool: PgPool) {
    let w = world(pool).await;
    let position = |v: &Value| v["position"].as_i64().unwrap();
    let mut tops = Vec::new();
    for title in ["A", "B", "C"] {
        let response = post(
            &w.http,
            &w.owner.token,
            &format!("/courses/{}/modules", w.course),
            json!({"title": title}),
        )
        .await;
        tops.push(position(&created(&response)));
    }
    assert_eq!(tops, [1, 2, 3]);

    let parent = module(&w, "Parent", None).await;
    for expected in [1, 2, 3] {
        let response = post(
            &w.http,
            &w.owner.token,
            &format!("/courses/{}/modules", w.course),
            json!({"title": "Sub", "parent_id": parent}),
        )
        .await;
        assert_eq!(
            position(&created(&response)),
            expected,
            "sub-modules count apart"
        );
    }
    for expected in [1, 2] {
        let response = post(
            &w.http,
            &w.owner.token,
            &format!("/modules/{parent}/items"),
            json!({"kind": "lesson", "title": "L"}),
        )
        .await;
        assert_eq!(position(&created(&response)), expected, "items count apart");
    }
    // A new top-level module still comes after the four existing ones.
    let response = post(
        &w.http,
        &w.owner.token,
        &format!("/courses/{}/modules", w.course),
        json!({"title": "E"}),
    )
    .await;
    assert_eq!(position(&created(&response)), 5);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_sub_module_needs_a_top_level_parent_of_the_same_course(pool: PgPool) {
    let w = world(pool).await;
    let top_level = module(&w, "Top", None).await;
    let sub = module(&w, "Sub", Some(top_level)).await;
    let other_course = create_course(&w.http, &w.owner.token, "Other").await;
    let foreign = module_in(&w, other_course, "Foreign", None).await;

    for parent in [sub, foreign, Uuid::now_v7()] {
        let response = post(
            &w.http,
            &w.owner.token,
            &format!("/courses/{}/modules", w.course),
            json!({"title": "Deeper", "parent_id": parent}),
        )
        .await;
        assert_eq!(first_error_field(&response), "parent_id", "{parent}");
    }
    let tree = tree(&w, &w.owner.token).await;
    assert_eq!(titles(&top(&tree, "Top")["submodules"]), ["Sub"]);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn lesson_bodies_are_stored_as_given_and_must_be_objects(pool: PgPool) {
    let w = world(pool).await;
    let module = module(&w, "M", None).await;
    let path = format!("/modules/{module}/items");

    let plain = created(
        &post(
            &w.http,
            &w.owner.token,
            &path,
            json!({"kind": "lesson", "title": "L"}),
        )
        .await,
    );
    assert_eq!(plain["body"], json!({}));
    assert_eq!(plain["kind"], "lesson");
    assert_eq!(plain["status"], "draft");

    let body = json!({"blocks": [{"type": "p", "text": "Hi"}], "n": 1.5, "nested": {"a": null}});
    let rich = created(
        &post(
            &w.http,
            &w.owner.token,
            &path,
            json!({"kind": "lesson", "title": "R", "body": body}),
        )
        .await,
    );
    assert_eq!(rich["body"], body);
    let fetched = ok(&get(
        &w.http,
        &w.owner.token,
        &format!("/items/{}", rich["id"].as_str().unwrap()),
    )
    .await);
    assert_eq!(fetched["body"], body);

    for bad in [json!([1, 2]), json!("text"), json!(7), json!(true)] {
        let response = post(
            &w.http,
            &w.owner.token,
            &path,
            json!({"kind": "lesson", "title": "Bad", "body": bad}),
        )
        .await;
        response.problem(422);
        let response = patch(
            &w.http,
            &w.owner.token,
            &format!("/items/{}", plain["id"].as_str().unwrap()),
            json!({"body": bad}),
        )
        .await;
        response.problem(422);
    }
    // Nothing was stored by the failed attempts.
    let tree = tree(&w, &w.owner.token).await;
    assert_eq!(top(&tree, "M")["items"].as_array().unwrap().len(), 2);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn titles_and_kinds_are_validated(pool: PgPool) {
    let w = world(pool).await;
    let path = format!("/courses/{}/modules", w.course);
    for title in ["", "   ", "line\nbreak"] {
        let response = post(&w.http, &w.owner.token, &path, json!({"title": title})).await;
        assert_eq!(first_error_field(&response), "title", "{title:?}");
    }
    let too_long = "x".repeat(201);
    post(&w.http, &w.owner.token, &path, json!({"title": too_long}))
        .await
        .problem(422);
    let trimmed = created(
        &post(
            &w.http,
            &w.owner.token,
            &path,
            json!({"title": "  Spaced  "}),
        )
        .await,
    );
    assert_eq!(trimmed["title"], "Spaced");

    let module = id_of(&trimmed);
    let items = format!("/modules/{module}/items");
    post(
        &w.http,
        &w.owner.token,
        &items,
        json!({"kind": "assignment", "title": "T"}),
    )
    .await
    .problem(422);
    post(&w.http, &w.owner.token, &items, json!({"title": "T"}))
        .await
        .problem(422);
    patch(
        &w.http,
        &w.owner.token,
        &format!("/modules/{module}"),
        json!({"title": " "}),
    )
    .await
    .problem(422);
}

// =============================================================================================
// Visibility (AGENTS §6)
// =============================================================================================

/// Builds one module per case. Each case is hidden from students for a different reason, except
/// `G`, which is open on every level.
struct Cases {
    /// The item in each case that a student must not reach.
    hidden: Vec<(&'static str, Uuid)>,
    open_direct: Uuid,
    open_in_sub: Uuid,
    draft_module: Uuid,
}

async fn build_cases(w: &World) -> Cases {
    let mut hidden = Vec::new();

    // A: draft top-level module with a published sub-module and a published item below it.
    let a = module(w, "A", None).await;
    let a_sub = module(w, "A-sub", Some(a)).await;
    publish_module(w, a_sub, false).await;
    let a_item = item(w, a_sub, "A-item").await;
    publish_item(w, a_item).await;
    hidden.push(("draft top-level module", a_item));

    // B: published module, draft sub-module, published item in it.
    let b = module(w, "B", None).await;
    publish_module(w, b, false).await;
    let b_sub = module(w, "B-sub", Some(b)).await;
    let b_item = item(w, b_sub, "B-item").await;
    publish_item(w, b_item).await;
    hidden.push(("draft sub-module", b_item));

    // C: published module with a draft item.
    let c = module(w, "C", None).await;
    publish_module(w, c, false).await;
    let c_item = item(w, c, "C-item").await;
    hidden.push(("draft item", c_item));

    // D: published module scheduled for later.
    let d = module(w, "D", None).await;
    let d_item = item(w, d, "D-item").await;
    publish_item(w, d_item).await;
    schedule_module(w, d, FUTURE).await;
    publish_module(w, d, false).await;
    hidden.push(("future module", d_item));

    // E: published sub-module scheduled for later.
    let e = module(w, "E", None).await;
    publish_module(w, e, false).await;
    let e_sub = module(w, "E-sub", Some(e)).await;
    let e_item = item(w, e_sub, "E-item").await;
    publish_item(w, e_item).await;
    schedule_module(w, e_sub, FUTURE).await;
    publish_module(w, e_sub, false).await;
    hidden.push(("future sub-module", e_item));

    // F: published item scheduled for later.
    let f = module(w, "F", None).await;
    publish_module(w, f, false).await;
    let f_item = item(w, f, "F-item").await;
    publish_item(w, f_item).await;
    schedule_item(w, f_item, FUTURE).await;
    hidden.push(("future item", f_item));

    // G: everything published and released, with a past publish_at on each level.
    let g = module(w, "G", None).await;
    let g_sub = module(w, "G-sub", Some(g)).await;
    let g_direct = item(w, g, "G-direct").await;
    let g_nested = item(w, g_sub, "G-nested").await;
    schedule_module(w, g, PAST).await;
    schedule_module(w, g_sub, PAST).await;
    schedule_item(w, g_nested, PAST).await;
    publish_module(w, g, true).await;

    Cases {
        hidden,
        open_direct: g_direct,
        open_in_sub: g_nested,
        draft_module: a,
    }
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn students_never_reach_content_that_is_not_released_on_every_level(pool: PgPool) {
    let w = world(pool).await;
    let cases = build_cases(&w).await;

    for (why, item) in &cases.hidden {
        let response = get(&w.http, &w.student.token, &format!("/items/{item}")).await;
        assert_eq!(response.status, 404, "{why}");
        response.problem(404);
    }
    for item in [cases.open_direct, cases.open_in_sub] {
        ok(&get(&w.http, &w.student.token, &format!("/items/{item}")).await);
    }

    let seen = tree(&w, &w.student.token).await;
    // A and D are absent; B, C, E and F are there, stripped of what is hidden.
    assert_eq!(titles(&seen["modules"]), ["B", "C", "E", "F", "G"]);
    assert_eq!(top(&seen, "B")["submodules"], json!([]), "draft sub-module");
    assert_eq!(top(&seen, "B")["items"], json!([]));
    assert_eq!(top(&seen, "C")["items"], json!([]), "draft item");
    assert_eq!(
        top(&seen, "E")["submodules"],
        json!([]),
        "future sub-module"
    );
    assert_eq!(top(&seen, "F")["items"], json!([]), "future item");
    let g = top(&seen, "G");
    assert_eq!(titles(&g["submodules"]), ["G-sub"]);
    assert_eq!(titles(&g["items"]), ["G-direct"]);
    assert_eq!(titles(&g["submodules"][0]["items"]), ["G-nested"]);

    // Nothing hidden leaks anywhere in the serialized tree.
    let raw = seen.to_string();
    for hidden in [
        "A-sub", "A-item", "B-sub", "B-item", "C-item", "D-item", "E-sub", "E-item", "F-item",
    ] {
        assert!(!raw.contains(hidden), "{hidden} leaked to a student");
    }

    // Staff see all of it, and can open every item.
    for token in [&w.owner.token, &w.assistant.token, &w.admin.token] {
        let full = tree(&w, token).await;
        assert_eq!(
            titles(&full["modules"]),
            ["A", "B", "C", "D", "E", "F", "G"]
        );
        assert_eq!(titles(&top(&full, "A")["submodules"]), ["A-sub"]);
        assert_eq!(titles(&top(&full, "B")["submodules"]), ["B-sub"]);
        assert_eq!(titles(&top(&full, "C")["items"]), ["C-item"]);
        assert_eq!(titles(&top(&full, "D")["items"]), ["D-item"]);
        assert_eq!(titles(&top(&full, "E")["submodules"]), ["E-sub"]);
        assert_eq!(titles(&top(&full, "F")["items"]), ["F-item"]);
        for (why, item) in &cases.hidden {
            let response = get(&w.http, token, &format!("/items/{item}")).await;
            assert_eq!(response.status, 200, "{why}");
        }
    }
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_student_sees_an_item_once_every_condition_holds(pool: PgPool) {
    let w = world(pool).await;
    let cases = build_cases(&w).await;
    let (_, nested_in_draft) = cases.hidden[0];
    let path = format!("/items/{nested_in_draft}");

    assert_eq!(get(&w.http, &w.student.token, &path).await.status, 404);
    // Publishing the draft module is the last missing condition.
    publish_module(&w, cases.draft_module, false).await;
    assert_eq!(get(&w.http, &w.student.token, &path).await.status, 200);
    let seen = tree(&w, &w.student.token).await;
    assert_eq!(titles(&top(&seen, "A")["submodules"]), ["A-sub"]);

    // Taking the release time back into the future hides it again, without unpublishing.
    schedule_module(&w, cases.draft_module, FUTURE).await;
    assert_eq!(get(&w.http, &w.student.token, &path).await.status, 404);
    // `publish_at: null` clears the schedule.
    ok(&patch(
        &w.http,
        &w.owner.token,
        &format!("/modules/{}", cases.draft_module),
        json!({"publish_at": null}),
    )
    .await);
    assert_eq!(get(&w.http, &w.student.token, &path).await.status, 200);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_student_sees_no_content_while_the_course_is_a_draft(pool: PgPool) {
    let w = world(pool).await;
    let draft_course = create_course(&w.http, &w.owner.token, "Draft course").await;
    put_member(
        &w.http,
        &w.owner.token,
        draft_course,
        w.student.id,
        "student",
    )
    .await;
    let module = module_in(&w, draft_course, "M", None).await;
    publish_module(&w, module, false).await;
    let item = item(&w, module, "L").await;
    publish_item(&w, item).await;

    assert_eq!(
        get(
            &w.http,
            &w.student.token,
            &format!("/courses/{draft_course}/content")
        )
        .await
        .status,
        404
    );
    assert_eq!(
        get(&w.http, &w.student.token, &format!("/items/{item}"))
            .await
            .status,
        404
    );
    assert_eq!(
        get(&w.http, &w.owner.token, &format!("/items/{item}"))
            .await
            .status,
        200
    );

    publish_course(&w.http, &w.owner.token, draft_course).await;
    assert_eq!(
        get(&w.http, &w.student.token, &format!("/items/{item}"))
            .await
            .status,
        200
    );
}

// =============================================================================================
// Permissions
// =============================================================================================

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_student_who_sees_a_lesson_still_cannot_change_it(pool: PgPool) {
    let w = world(pool).await;
    let module = module(&w, "M", None).await;
    publish_module(&w, module, false).await;
    let lesson = item(&w, module, "L").await;
    publish_item(&w, lesson).await;
    let token = &w.student.token;

    for response in [
        patch(
            &w.http,
            token,
            &format!("/items/{lesson}"),
            json!({"title": "X"}),
        )
        .await,
        delete(&w.http, token, &format!("/items/{lesson}")).await,
        post(
            &w.http,
            token,
            &format!("/items/{lesson}/publish"),
            json!(null),
        )
        .await,
        patch(
            &w.http,
            token,
            &format!("/modules/{module}"),
            json!({"title": "X"}),
        )
        .await,
        delete(&w.http, token, &format!("/modules/{module}")).await,
        post(
            &w.http,
            token,
            &format!("/modules/{module}/publish"),
            json!({"cascade": true}),
        )
        .await,
        post(
            &w.http,
            token,
            &format!("/modules/{module}/items"),
            json!({"kind": "lesson", "title": "T"}),
        )
        .await,
        put(
            &w.http,
            token,
            &format!("/modules/{module}/items/order"),
            json!({"ids": [lesson]}),
        )
        .await,
        post(
            &w.http,
            token,
            &format!("/courses/{}/modules", w.course),
            json!({"title": "T"}),
        )
        .await,
        put(
            &w.http,
            token,
            &format!("/courses/{}/modules/order", w.course),
            json!({"ids": [module]}),
        )
        .await,
    ] {
        response.problem(403);
    }
    // Nothing changed.
    let item = ok(&get(&w.http, &w.owner.token, &format!("/items/{lesson}")).await);
    assert_eq!(item["title"], "L");
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_student_gets_404_not_403_for_content_they_cannot_see(pool: PgPool) {
    let w = world(pool).await;
    let module = module(&w, "M", None).await;
    let draft = item(&w, module, "Draft").await;
    let token = &w.student.token;
    patch(
        &w.http,
        token,
        &format!("/items/{draft}"),
        json!({"title": "X"}),
    )
    .await
    .problem(404);
    delete(&w.http, token, &format!("/items/{draft}"))
        .await
        .problem(404);
    patch(
        &w.http,
        token,
        &format!("/modules/{module}"),
        json!({"title": "X"}),
    )
    .await
    .problem(404);
    post(
        &w.http,
        token,
        &format!("/modules/{module}/items"),
        json!({"kind": "lesson", "title": "T"}),
    )
    .await
    .problem(404);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_non_member_sees_and_changes_nothing(pool: PgPool) {
    let w = world(pool).await;
    let module = module(&w, "M", None).await;
    publish_module(&w, module, false).await;
    let lesson = item(&w, module, "L").await;
    publish_item(&w, lesson).await;
    let token = &w.stranger.token;

    for response in [
        get(&w.http, token, &format!("/courses/{}/content", w.course)).await,
        get(&w.http, token, &format!("/items/{lesson}")).await,
        patch(
            &w.http,
            token,
            &format!("/items/{lesson}"),
            json!({"title": "X"}),
        )
        .await,
        delete(&w.http, token, &format!("/items/{lesson}")).await,
        post(
            &w.http,
            token,
            &format!("/items/{lesson}/publish"),
            json!(null),
        )
        .await,
        patch(
            &w.http,
            token,
            &format!("/modules/{module}"),
            json!({"title": "X"}),
        )
        .await,
        delete(&w.http, token, &format!("/modules/{module}")).await,
        post(
            &w.http,
            token,
            &format!("/modules/{module}/publish"),
            json!({"cascade": false}),
        )
        .await,
        post(
            &w.http,
            token,
            &format!("/modules/{module}/items"),
            json!({"kind": "lesson", "title": "T"}),
        )
        .await,
        post(
            &w.http,
            token,
            &format!("/courses/{}/modules", w.course),
            json!({"title": "T"}),
        )
        .await,
        put(
            &w.http,
            token,
            &format!("/courses/{}/modules/order", w.course),
            json!({"ids": [module]}),
        )
        .await,
        put(
            &w.http,
            token,
            &format!("/modules/{module}/items/order"),
            json!({"ids": [lesson]}),
        )
        .await,
    ] {
        response.problem(404);
    }
    // Everything needs a login.
    for response in [
        w.http.get(&format!("/items/{lesson}")).send().await,
        w.http
            .get(&format!("/courses/{}/content", w.course))
            .send()
            .await,
        w.http
            .post(&format!("/modules/{module}/publish"))
            .json(&json!({"cascade": true}))
            .send()
            .await,
    ] {
        response.problem(401);
    }
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn an_unknown_id_and_an_invisible_one_give_the_same_404(pool: PgPool) {
    let w = world(pool).await;
    let hidden_module = module(&w, "Hidden", None).await;
    let hidden_item = item(&w, hidden_module, "Hidden item").await;
    let unknown = Uuid::now_v7();

    // Pairs of (request for a row that exists but the caller cannot see, same request for an
    // id that does not exist). The responses must be indistinguishable.
    let stranger = &w.stranger.token;
    let student = &w.student.token;
    let mut pairs = vec![
        (
            get(&w.http, stranger, &format!("/items/{hidden_item}")).await,
            get(&w.http, stranger, &format!("/items/{unknown}")).await,
        ),
        (
            patch(
                &w.http,
                stranger,
                &format!("/modules/{hidden_module}"),
                json!({"title": "X"}),
            )
            .await,
            patch(
                &w.http,
                stranger,
                &format!("/modules/{unknown}"),
                json!({"title": "X"}),
            )
            .await,
        ),
        (
            post(
                &w.http,
                stranger,
                &format!("/modules/{hidden_module}/publish"),
                json!({"cascade": true}),
            )
            .await,
            post(
                &w.http,
                stranger,
                &format!("/modules/{unknown}/publish"),
                json!({"cascade": true}),
            )
            .await,
        ),
        (
            get(&w.http, stranger, &format!("/courses/{}/content", w.course)).await,
            get(&w.http, stranger, &format!("/courses/{unknown}/content")).await,
        ),
        // A student in the course, for a draft they may not open.
        (
            get(&w.http, student, &format!("/items/{hidden_item}")).await,
            get(&w.http, student, &format!("/items/{unknown}")).await,
        ),
        (
            delete(&w.http, student, &format!("/modules/{hidden_module}")).await,
            delete(&w.http, student, &format!("/modules/{unknown}")).await,
        ),
    ];
    for (i, (invisible, absent)) in pairs.drain(..).enumerate() {
        assert_eq!(invisible.status, 404, "pair {i}");
        assert_eq!(invisible.status, absent.status, "pair {i}");
        assert_eq!(
            invisible.header("content-type"),
            absent.header("content-type"),
            "pair {i}"
        );
        assert_eq!(invisible.body, absent.body, "pair {i}: bodies differ");
    }
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn malformed_ids_are_400_and_unknown_ids_404(pool: PgPool) {
    let w = world(pool).await;
    let token = &w.owner.token;
    for path in ["/items/not-a-uuid", "/courses/x/content"] {
        get(&w.http, token, path).await.problem(400);
    }
    post(
        &w.http,
        token,
        "/modules/x/publish",
        json!({"cascade": true}),
    )
    .await
    .problem(400);
    patch(&w.http, token, "/modules/1234", json!({"title": "T"}))
        .await
        .problem(400);
    delete(&w.http, token, "/items/zzz").await.problem(400);
    put(
        &w.http,
        token,
        "/courses/x/modules/order",
        json!({"ids": []}),
    )
    .await
    .problem(400);
    put(&w.http, token, "/modules/x/items/order", json!({"ids": []}))
        .await
        .problem(400);

    let unknown = Uuid::now_v7();
    get(&w.http, token, &format!("/items/{unknown}"))
        .await
        .problem(404);
    get(&w.http, token, &format!("/courses/{unknown}/content"))
        .await
        .problem(404);
    post(
        &w.http,
        token,
        &format!("/courses/{unknown}/modules"),
        json!({"title": "T"}),
    )
    .await
    .problem(404);
    post(
        &w.http,
        token,
        &format!("/modules/{unknown}/items"),
        json!({"kind": "lesson", "title": "T"}),
    )
    .await
    .problem(404);
    delete(&w.http, token, &format!("/modules/{unknown}"))
        .await
        .problem(404);
    post(
        &w.http,
        token,
        &format!("/items/{unknown}/publish"),
        json!(null),
    )
    .await
    .problem(404);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn an_assistant_runs_the_whole_tree(pool: PgPool) {
    let w = world(pool).await;
    let token = &w.assistant.token;
    let first = module_as(&w, token, "First", None).await;
    let second = module_as(&w, token, "Second", None).await;
    let sub = module_as(&w, token, "Sub", Some(first)).await;
    let lesson = id_of(&created(
        &post(
            &w.http,
            token,
            &format!("/modules/{sub}/items"),
            json!({"kind": "lesson", "title": "L"}),
        )
        .await,
    ));
    let other = id_of(&created(
        &post(
            &w.http,
            token,
            &format!("/modules/{sub}/items"),
            json!({"kind": "lesson", "title": "M"}),
        )
        .await,
    ));

    assert_eq!(
        ok(&patch(
            &w.http,
            token,
            &format!("/modules/{first}"),
            json!({"title": "Renamed"})
        )
        .await)["title"],
        "Renamed"
    );
    assert_eq!(
        ok(&patch(
            &w.http,
            token,
            &format!("/items/{lesson}"),
            json!({"body": {"a": 1}})
        )
        .await)["body"],
        json!({"a": 1})
    );
    no_content(
        &put(
            &w.http,
            token,
            &format!("/courses/{}/modules/order", w.course),
            json!({"ids": [second, first]}),
        )
        .await,
    );
    no_content(
        &put(
            &w.http,
            token,
            &format!("/modules/{sub}/items/order"),
            json!({"ids": [other, lesson]}),
        )
        .await,
    );
    assert_eq!(
        ok(&post(
            &w.http,
            token,
            &format!("/modules/{first}/publish"),
            json!({"cascade": true})
        )
        .await)["status"],
        "published"
    );
    assert_eq!(
        ok(&post(
            &w.http,
            token,
            &format!("/items/{lesson}/publish"),
            json!(null)
        )
        .await)["status"],
        "published"
    );
    no_content(&delete(&w.http, token, &format!("/items/{other}")).await);
    no_content(&delete(&w.http, token, &format!("/modules/{second}")).await);

    let full = tree(&w, token).await;
    assert_eq!(titles(&full["modules"]), ["Renamed"]);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn content_of_an_archived_course_is_editable_by_staff(pool: PgPool) {
    // ManageContent ignores the course status, so archived content stays editable; archiving
    // gets its own ADR. This pins the behavior so a later change is deliberate.
    let w = world(pool).await;
    let module = module(&w, "M", None).await;
    sqlx::query!(
        "UPDATE teaching.courses SET status = 'archived' WHERE id = $1",
        w.course
    )
    .execute(&w.pool)
    .await
    .unwrap();
    ok(&patch(
        &w.http,
        &w.owner.token,
        &format!("/modules/{module}"),
        json!({"title": "Edited"}),
    )
    .await);
    get(
        &w.http,
        &w.student.token,
        &format!("/courses/{}/content", w.course),
    )
    .await
    .problem(404);
}

// =============================================================================================
// Publishing
// =============================================================================================

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn publishing_a_module_requires_an_explicit_cascade(pool: PgPool) {
    let w = world(pool).await;
    let module = module(&w, "M", None).await;
    let path = format!("/modules/{module}/publish");
    post(&w.http, &w.owner.token, &path, json!({}))
        .await
        .problem(422);
    post(&w.http, &w.owner.token, &path, json!({"cascade": "yes"}))
        .await
        .problem(422);
    let tree = tree(&w, &w.owner.token).await;
    assert_eq!(top(&tree, "M")["status"], "draft");
}

/// M (draft) holds the sub-module S, the direct item D and, inside S, the item N.
async fn publish_fixture(w: &World) -> (Uuid, Uuid, Uuid, Uuid) {
    let m = module(w, "M", None).await;
    let s = module(w, "S", Some(m)).await;
    let d = item(w, m, "D").await;
    let n = item(w, s, "N").await;
    (m, s, d, n)
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn cascade_false_publishes_only_the_module(pool: PgPool) {
    let w = world(pool).await;
    let (m, ..) = publish_fixture(&w).await;
    let published = publish_module(&w, m, false).await;
    assert_eq!(published["status"], "published");

    let tree = tree(&w, &w.owner.token).await;
    let node = top(&tree, "M");
    assert_eq!(node["status"], "published");
    assert_eq!(node["submodules"][0]["status"], "draft");
    assert_eq!(node["items"][0]["status"], "draft");
    assert_eq!(node["submodules"][0]["items"][0]["status"], "draft");
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn cascade_true_publishes_the_sub_modules_and_every_draft_item_below(pool: PgPool) {
    let w = world(pool).await;
    let (m, ..) = publish_fixture(&w).await;
    publish_module(&w, m, true).await;

    let tree = tree(&w, &w.owner.token).await;
    let node = top(&tree, "M");
    assert_eq!(node["status"], "published");
    assert_eq!(node["submodules"][0]["status"], "published");
    assert_eq!(node["items"][0]["status"], "published");
    assert_eq!(node["submodules"][0]["items"][0]["status"], "published");

    // A student now sees all of it.
    let seen = crate::tree(&w, &w.student.token).await;
    assert_eq!(titles(&top(&seen, "M")["submodules"][0]["items"]), ["N"]);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn cascade_from_a_sub_module_covers_only_its_own_items(pool: PgPool) {
    let w = world(pool).await;
    let (_, s, d, n) = publish_fixture(&w).await;
    publish_module(&w, s, true).await;
    assert_eq!(
        ok(&get(&w.http, &w.owner.token, &format!("/items/{n}")).await)["status"],
        "published"
    );
    assert_eq!(
        ok(&get(&w.http, &w.owner.token, &format!("/items/{d}")).await)["status"],
        "draft"
    );
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn publishing_twice_changes_nothing(pool: PgPool) {
    let w = world(pool).await;
    let (m, s, d, _) = publish_fixture(&w).await;

    let first = publish_module(&w, m, true).await;
    let before = tree(&w, &w.owner.token).await;
    let second = publish_module(&w, m, true).await;
    let third = publish_module(&w, m, false).await;
    assert_eq!(first, second);
    assert_eq!(first, third);
    assert_eq!(first["updated_at"], second["updated_at"]);
    assert_eq!(before, tree(&w, &w.owner.token).await);
    let sub_before = publish_module(&w, s, false).await;
    assert_eq!(sub_before["status"], "published");

    let item_first = publish_item(&w, d).await;
    let item_second = publish_item(&w, d).await;
    assert_eq!(item_first, item_second);
    assert_eq!(item_first["status"], "published");
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn publishing_never_touches_publish_at(pool: PgPool) {
    let w = world(pool).await;
    let (m, s, d, n) = publish_fixture(&w).await;
    schedule_module(&w, m, FUTURE).await;
    schedule_module(&w, s, FUTURE).await;
    schedule_item(&w, d, FUTURE).await;
    schedule_item(&w, n, PAST).await;

    let published = publish_module(&w, m, true).await;
    assert_eq!(published["publish_at"], json!(FUTURE));
    let tree = tree(&w, &w.owner.token).await;
    let node = top(&tree, "M");
    assert_eq!(node["submodules"][0]["publish_at"], FUTURE);
    assert_eq!(node["items"][0]["publish_at"], FUTURE);
    assert_eq!(node["submodules"][0]["items"][0]["publish_at"], PAST);
    let lesson = publish_item(&w, d).await;
    assert_eq!(lesson["publish_at"], FUTURE);
    // Still scheduled for the future, so students see none of it.
    assert_eq!(
        crate::tree(&w, &w.student.token).await["modules"],
        json!([])
    );
}

// =============================================================================================
// Editing and deleting
// =============================================================================================

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn modules_and_lessons_are_edited_through_patch(pool: PgPool) {
    let w = world(pool).await;
    let m = module(&w, "Old", None).await;
    let l = item(&w, m, "Old lesson").await;
    let token = &w.owner.token;

    let renamed = ok(&patch(
        &w.http,
        token,
        &format!("/modules/{m}"),
        json!({"title": " New "}),
    )
    .await);
    assert_eq!(renamed["title"], "New");
    assert_eq!(renamed["publish_at"], Value::Null);
    let scheduled = ok(&patch(
        &w.http,
        token,
        &format!("/modules/{m}"),
        json!({"publish_at": FUTURE}),
    )
    .await);
    assert_eq!(scheduled["publish_at"], FUTURE);
    assert_eq!(scheduled["title"], "New", "an absent field is kept");
    let cleared = ok(&patch(
        &w.http,
        token,
        &format!("/modules/{m}"),
        json!({"publish_at": null}),
    )
    .await);
    assert_eq!(cleared["publish_at"], Value::Null);
    patch(
        &w.http,
        token,
        &format!("/modules/{m}"),
        json!({"publish_at": "tomorrow"}),
    )
    .await
    .problem(422);

    let edited = ok(&patch(
        &w.http,
        token,
        &format!("/items/{l}"),
        json!({"title": "T", "body": {"x": [1]}}),
    )
    .await);
    assert_eq!(
        (edited["title"].as_str(), &edited["body"]),
        (Some("T"), &json!({"x": [1]}))
    );
    let retitled = ok(&patch(
        &w.http,
        token,
        &format!("/items/{l}"),
        json!({"title": "U"}),
    )
    .await);
    assert_eq!(
        retitled["body"],
        json!({"x": [1]}),
        "an absent body is kept"
    );
    let emptied = ok(&patch(&w.http, token, &format!("/items/{l}"), json!({"body": {}})).await);
    assert_eq!(emptied["body"], json!({}));
    let no_change = ok(&patch(&w.http, token, &format!("/items/{l}"), json!({})).await);
    assert_eq!(
        no_change["updated_at"], emptied["updated_at"],
        "an empty patch writes nothing"
    );
    patch(&w.http, token, &format!("/items/{l}"), json!({"title": ""}))
        .await
        .problem(422);
    // The parent cannot be changed through PATCH: the field is not part of the request.
    let other = module(&w, "Other", None).await;
    let still = ok(&patch(
        &w.http,
        token,
        &format!("/modules/{m}"),
        json!({"parent_id": other}),
    )
    .await);
    assert_eq!(still["parent_id"], Value::Null);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn deleting_a_module_removes_its_sub_modules_and_items(pool: PgPool) {
    let w = world(pool).await;
    let (m, s, d, n) = publish_fixture(&w).await;
    let keep = module(&w, "Keep", None).await;
    no_content(&delete(&w.http, &w.owner.token, &format!("/modules/{m}")).await);

    for id in [d, n] {
        get(&w.http, &w.owner.token, &format!("/items/{id}"))
            .await
            .problem(404);
    }
    delete(&w.http, &w.owner.token, &format!("/modules/{s}"))
        .await
        .problem(404);
    let tree = tree(&w, &w.owner.token).await;
    assert_eq!(titles(&tree["modules"]), ["Keep"]);
    assert_eq!(top(&tree, "Keep")["id"], json!(keep));
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn deleting_a_sub_module_leaves_its_parent(pool: PgPool) {
    let w = world(pool).await;
    let (m, s, d, n) = publish_fixture(&w).await;
    no_content(&delete(&w.http, &w.owner.token, &format!("/modules/{s}")).await);
    get(&w.http, &w.owner.token, &format!("/items/{n}"))
        .await
        .problem(404);
    ok(&get(&w.http, &w.owner.token, &format!("/items/{d}")).await);
    let tree = tree(&w, &w.owner.token).await;
    assert_eq!(top(&tree, "M")["id"], json!(m));
    assert_eq!(top(&tree, "M")["submodules"], json!([]));
}

// =============================================================================================
// Order
// =============================================================================================

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn modules_sub_modules_and_items_can_be_swapped(pool: PgPool) {
    let w = world(pool).await;
    let a = module(&w, "A", None).await;
    let b = module(&w, "B", None).await;
    let a1 = module(&w, "A1", Some(a)).await;
    let a2 = module(&w, "A2", Some(a)).await;
    let i1 = item(&w, a, "I1").await;
    let i2 = item(&w, a, "I2").await;
    let i3 = item(&w, a, "I3").await;
    let token = &w.owner.token;

    no_content(
        &put(
            &w.http,
            token,
            &format!("/courses/{}/modules/order", w.course),
            json!({"ids": [b, a]}),
        )
        .await,
    );
    no_content(
        &put(
            &w.http,
            token,
            &format!("/courses/{}/modules/order", w.course),
            json!({"parent_id": a, "ids": [a2, a1]}),
        )
        .await,
    );
    no_content(
        &put(
            &w.http,
            token,
            &format!("/modules/{a}/items/order"),
            json!({"ids": [i3, i1, i2]}),
        )
        .await,
    );

    let tree = tree(&w, token).await;
    assert_eq!(titles(&tree["modules"]), ["B", "A"]);
    let node = top(&tree, "A");
    assert_eq!(titles(&node["submodules"]), ["A2", "A1"]);
    assert_eq!(titles(&node["items"]), ["I3", "I1", "I2"]);
    let positions: Vec<_> = node["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["position"].as_i64().unwrap())
        .collect();
    assert_eq!(positions, [1, 2, 3]);
    assert_eq!(tree["modules"][0]["position"], 1);
    assert_eq!(node["position"], 2);
    assert_eq!(node["submodules"][0]["position"], 1);

    // Reordering twice with the same list is a no-op that still succeeds.
    no_content(
        &put(
            &w.http,
            token,
            &format!("/modules/{a}/items/order"),
            json!({"ids": [i3, i1, i2]}),
        )
        .await,
    );
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_reorder_must_list_exactly_the_current_siblings(pool: PgPool) {
    let w = world(pool).await;
    let a = module(&w, "A", None).await;
    let b = module(&w, "B", None).await;
    let c = module(&w, "C", None).await;
    let s1 = module(&w, "S1", Some(a)).await;
    let s2 = module(&w, "S2", Some(a)).await;
    let i1 = item(&w, a, "I1").await;
    let i2 = item(&w, a, "I2").await;
    let other_course = create_course(&w.http, &w.owner.token, "Other").await;
    let foreign_module = module_in(&w, other_course, "X", None).await;
    let foreign_item = item(&w, foreign_module, "XI").await;
    let token = &w.owner.token;
    let top_path = format!("/courses/{}/modules/order", w.course);
    let items_path = format!("/modules/{a}/items/order");

    // Top level.
    for (why, body) in [
        ("missing", json!({"ids": [a, b]})),
        ("extra", json!({"ids": [a, b, c, Uuid::now_v7()]})),
        ("duplicate", json!({"ids": [a, a, b, c]})),
        ("duplicate in place of one", json!({"ids": [a, a, b]})),
        ("foreign", json!({"ids": [a, b, foreign_module]})),
        ("sub-modules without a parent", json!({"ids": [s1, s2]})),
        ("mixed levels", json!({"ids": [a, b, c, s1]})),
        ("empty", json!({"ids": []})),
    ] {
        let response = put(&w.http, token, &top_path, body).await;
        assert_eq!(first_error_field(&response), "ids", "{why}");
    }
    // Sub-modules.
    for (why, body) in [
        (
            "top-level ids under a parent",
            json!({"parent_id": a, "ids": [a, b, c]}),
        ),
        ("missing", json!({"parent_id": a, "ids": [s1]})),
        (
            "another parent's sequence",
            json!({"parent_id": b, "ids": [s1, s2]}),
        ),
    ] {
        let response = put(&w.http, token, &top_path, body).await;
        assert_eq!(first_error_field(&response), "ids", "{why}");
    }
    // A parent that cannot hold sub-modules.
    for parent in [s1, foreign_module, Uuid::now_v7()] {
        let response = put(
            &w.http,
            token,
            &top_path,
            json!({"parent_id": parent, "ids": []}),
        )
        .await;
        assert_eq!(first_error_field(&response), "parent_id", "{parent}");
    }
    // Items.
    for (why, body) in [
        ("missing", json!({"ids": [i1]})),
        ("extra", json!({"ids": [i1, i2, Uuid::now_v7()]})),
        ("duplicate", json!({"ids": [i1, i1]})),
        ("foreign", json!({"ids": [i1, foreign_item]})),
        ("a module id", json!({"ids": [i1, i2, s1]})),
    ] {
        let response = put(&w.http, token, &items_path, body).await;
        assert_eq!(first_error_field(&response), "ids", "{why}");
    }

    // Every refusal left the order alone.
    let tree = tree(&w, token).await;
    assert_eq!(titles(&tree["modules"]), ["A", "B", "C"]);
    assert_eq!(titles(&top(&tree, "A")["submodules"]), ["S1", "S2"]);
    assert_eq!(titles(&top(&tree, "A")["items"]), ["I1", "I2"]);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn deleting_leaves_gaps_and_later_creates_still_append(pool: PgPool) {
    let w = world(pool).await;
    let m = module(&w, "M", None).await;
    let first = item(&w, m, "One").await;
    let middle = item(&w, m, "Two").await;
    item(&w, m, "Three").await;
    no_content(&delete(&w.http, &w.owner.token, &format!("/items/{middle}")).await);

    let tree = tree(&w, &w.owner.token).await;
    let positions: Vec<_> = top(&tree, "M")["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["position"].as_i64().unwrap())
        .collect();
    assert_eq!(positions, [1, 3], "the gap stays");
    let appended = created(
        &post(
            &w.http,
            &w.owner.token,
            &format!("/modules/{m}/items"),
            json!({"kind": "lesson", "title": "Four"}),
        )
        .await,
    );
    assert_eq!(appended["position"], 4);
    // The remaining items can be reordered although positions are not dense.
    let ids: Vec<Uuid> = {
        let tree = crate::tree(&w, &w.owner.token).await;
        top(&tree, "M")["items"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .map(id_of)
            .collect()
    };
    assert!(ids.contains(&first));
    no_content(
        &put(
            &w.http,
            &w.owner.token,
            &format!("/modules/{m}/items/order"),
            json!({"ids": ids}),
        )
        .await,
    );
    let tree = crate::tree(&w, &w.owner.token).await;
    assert_eq!(titles(&top(&tree, "M")["items"]), ["Four", "Three", "One"]);
    let positions: Vec<_> = top(&tree, "M")["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["position"].as_i64().unwrap())
        .collect();
    assert_eq!(
        positions,
        [1, 2, 3],
        "a reorder makes positions dense again"
    );
}

// =============================================================================================
// Audit
// =============================================================================================

/// The ids of the course's top-level modules in their current order.
async fn top_level_ids(w: &World) -> Vec<Uuid> {
    tree(w, &w.owner.token).await["modules"]
        .as_array()
        .unwrap()
        .iter()
        .map(id_of)
        .collect()
}

/// Runs the same sequence of writes as `token` and returns the ids it touched.
async fn write_everything(w: &World, token: &str) -> (Uuid, Uuid, Uuid, Uuid, Uuid) {
    let courses = format!("/courses/{}/modules", w.course);
    let m = id_of(&created(
        &post(&w.http, token, &courses, json!({"title": "M"})).await,
    ));
    let s = id_of(&created(
        &post(
            &w.http,
            token,
            &courses,
            json!({"title": "S", "parent_id": m}),
        )
        .await,
    ));
    let i = id_of(&created(
        &post(
            &w.http,
            token,
            &format!("/modules/{m}/items"),
            json!({"kind": "lesson", "title": "I"}),
        )
        .await,
    ));
    let j = id_of(&created(
        &post(
            &w.http,
            token,
            &format!("/modules/{m}/items"),
            json!({"kind": "lesson", "title": "J"}),
        )
        .await,
    ));
    ok(&patch(
        &w.http,
        token,
        &format!("/modules/{m}"),
        json!({"title": "M2"}),
    )
    .await);
    ok(&patch(
        &w.http,
        token,
        &format!("/items/{i}"),
        json!({"title": "I2"}),
    )
    .await);
    ok(&post(&w.http, token, &format!("/items/{i}/publish"), json!(null)).await);
    ok(&post(
        &w.http,
        token,
        &format!("/modules/{m}/publish"),
        json!({"cascade": true}),
    )
    .await);
    no_content(
        &put(
            &w.http,
            token,
            &format!("{courses}/order"),
            json!({"ids": top_level_ids(w).await}),
        )
        .await,
    );
    no_content(
        &put(
            &w.http,
            token,
            &format!("{courses}/order"),
            json!({"parent_id": m, "ids": [s]}),
        )
        .await,
    );
    no_content(
        &put(
            &w.http,
            token,
            &format!("/modules/{m}/items/order"),
            json!({"ids": [j, i]}),
        )
        .await,
    );
    no_content(&delete(&w.http, token, &format!("/items/{j}")).await);
    no_content(&delete(&w.http, token, &format!("/modules/{s}")).await);
    (m, s, i, j, m)
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn admin_writes_are_audited_with_the_course_id_and_staff_writes_are_not(pool: PgPool) {
    let w = world(pool).await;
    write_everything(&w, &w.owner.token).await;
    write_everything(&w, &w.assistant.token).await;
    assert_eq!(
        audit_rows(&w.pool).await,
        [],
        "owner and assistant write no audit rows"
    );
    // Reads by an admin are not audited either.
    tree(&w, &w.admin.token).await;

    let (m, s, i, j, _) = write_everything(&w, &w.admin.token).await;
    let rows = audit_rows(&w.pool).await;
    // The module created by the admin, its sub-module, the two items; then each write in order.
    let summary: Vec<_> = rows
        .iter()
        .map(|(a, t, id, _)| (a.as_str(), t.as_str(), *id))
        .collect();
    assert_eq!(
        summary,
        [
            ("content.module.create", "module", m),
            ("content.module.create", "module", s),
            ("content.item.create", "item", i),
            ("content.item.create", "item", j),
            ("content.module.update", "module", m),
            ("content.item.update", "item", i),
            ("content.item.publish", "item", i),
            ("content.module.publish", "module", m),
            ("content.module.reorder", "course", w.course),
            ("content.module.reorder", "module", m),
            ("content.item.reorder", "module", m),
            ("content.item.delete", "item", j),
            ("content.module.delete", "module", s),
        ]
    );
    for (_, _, _, meta) in &rows {
        assert_eq!(meta, &json!({"course_id": w.course.to_string()}));
    }
    let actors: Vec<Uuid> = sqlx::query_scalar!("SELECT DISTINCT actor_id FROM events.audit_log")
        .fetch_all(&w.pool)
        .await
        .unwrap();
    assert_eq!(actors, [w.admin.id]);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn writes_that_change_nothing_or_fail_are_not_audited(pool: PgPool) {
    let w = world(pool).await;
    let m = module(&w, "M", None).await;
    let token = &w.admin.token;
    let before = audit_rows(&w.pool).await.len();

    ok(&patch(&w.http, token, &format!("/modules/{m}"), json!({})).await);
    patch(
        &w.http,
        token,
        &format!("/modules/{m}"),
        json!({"title": ""}),
    )
    .await
    .problem(422);
    put(
        &w.http,
        token,
        &format!("/courses/{}/modules/order", w.course),
        json!({"ids": []}),
    )
    .await
    .problem(422);
    post(&w.http, token, &format!("/modules/{m}/publish"), json!({}))
        .await
        .problem(422);
    assert_eq!(audit_rows(&w.pool).await.len(), before);

    publish_module(&w, m, false).await;
    let after_first = audit_rows(&w.pool).await.len();
    // The owner published it, so the admin's repeat changes nothing and writes nothing.
    ok(&post(
        &w.http,
        token,
        &format!("/modules/{m}/publish"),
        json!({"cascade": true}),
    )
    .await);
    assert_eq!(audit_rows(&w.pool).await.len(), after_first);
}

// =============================================================================================
// OpenAPI
// =============================================================================================

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn every_content_endpoint_is_in_the_openapi_document(pool: PgPool) {
    let base = common::spawn(pool).await;
    let response = reqwest::get(format!("{base}/api/v1/openapi.json"))
        .await
        .unwrap();
    let doc: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    for (path, method) in [
        ("/api/v1/courses/{id}/modules", "post"),
        ("/api/v1/courses/{id}/content", "get"),
        ("/api/v1/modules/{id}", "patch"),
        ("/api/v1/modules/{id}", "delete"),
        ("/api/v1/courses/{id}/modules/order", "put"),
        ("/api/v1/modules/{id}/publish", "post"),
        ("/api/v1/modules/{id}/items", "post"),
        ("/api/v1/items/{id}", "get"),
        ("/api/v1/items/{id}", "patch"),
        ("/api/v1/items/{id}", "delete"),
        ("/api/v1/modules/{id}/items/order", "put"),
        ("/api/v1/items/{id}/publish", "post"),
    ] {
        let operation = &doc["paths"][path][method];
        assert!(operation.is_object(), "{method} {path} missing");
        assert_eq!(operation["tags"], json!(["content"]), "{method} {path}");
        assert!(
            operation["security"].is_array(),
            "{method} {path} has no security"
        );
    }
    // The publish request makes `cascade` required.
    let publish = &doc["components"]["schemas"]["PublishModuleRequest"];
    assert_eq!(publish["required"], json!(["cascade"]));
}
