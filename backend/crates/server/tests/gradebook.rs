mod common;

use ::common::Role;
use common::{
    Http, Resp, create_course, db_test, id_of, publish_course, put_member, seed_and_login,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

const FUTURE: &str = "2099-01-01T00:00:00Z";

struct User {
    id: Uuid,
    token: String,
}

async fn user(pool: &PgPool, http: &Http, email: &str, role: Role) -> User {
    let (id, token) = seed_and_login(pool, http, email, role).await;
    User { id, token }
}

/// A published course with an owner, an assistant and a student, plus a teacher who is not a
/// member, a student of no course and an admin. One top-level module holds the assignments.
struct World {
    http: Http,
    pool: PgPool,
    owner: User,
    assistant: User,
    student: User,
    stranger: User,
    outsider: User,
    admin: User,
    course: Uuid,
    module: Uuid,
}

async fn world(pool: PgPool) -> World {
    let http = Http::new(&common::spawn(pool.clone()).await);
    let owner = user(&pool, &http, "owner@x.edu", Role::Teacher).await;
    let assistant = user(&pool, &http, "assistant@x.edu", Role::Student).await;
    let student = user(&pool, &http, "student@x.edu", Role::Student).await;
    let stranger = user(&pool, &http, "stranger@x.edu", Role::Teacher).await;
    let outsider = user(&pool, &http, "outsider@x.edu", Role::Student).await;
    let admin = user(&pool, &http, "admin@x.edu", Role::Admin).await;
    let course = create_course(&http, &owner.token, "Algebra 7-A").await;
    put_member(&http, &owner.token, course, assistant.id, "assistant").await;
    put_member(&http, &owner.token, course, student.id, "student").await;
    publish_course(&http, &owner.token, course).await;
    let mut w = World {
        http,
        pool,
        owner,
        assistant,
        student,
        stranger,
        outsider,
        admin,
        course,
        module: Uuid::nil(),
    };
    w.module = module(&w, "M", None).await;
    w
}

// --- request helpers ---------------------------------------------------------------------------

async fn call(
    http: &Http,
    method: &str,
    token: Option<&str>,
    path: &str,
    body: Option<Value>,
) -> Resp {
    let mut request = match method {
        "GET" => http.get(path),
        "POST" => http.post(path),
        "PUT" => http.put(path),
        "PATCH" => http.patch(path),
        "DELETE" => http.delete(path),
        other => panic!("{other}"),
    };
    if let Some(token) = token {
        request = request.bearer(token);
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    request.send().await
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

async fn module(w: &World, title: &str, parent: Option<Uuid>) -> Uuid {
    let response = call(
        &w.http,
        "POST",
        Some(&w.owner.token),
        &format!("/courses/{}/modules", w.course),
        Some(json!({"title": title, "parent_id": parent})),
    )
    .await;
    assert_eq!(
        response.status,
        201,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    id_of(&response.json())
}

/// A draft assignment of the given type in `module`.
async fn assignment_in(w: &World, module: Uuid, title: &str, kind: &str, points: &str) -> Uuid {
    let response = call(
        &w.http,
        "POST",
        Some(&w.owner.token),
        &format!("/modules/{module}/items"),
        Some(json!({
            "kind": "assignment",
            "title": title,
            "assignment": {"type": kind, "max_points": points},
        })),
    )
    .await;
    assert_eq!(
        response.status,
        201,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    id_of(&response.json())
}

/// A published assignment worth 20 points in the main module, which is published too.
async fn open_assignment(w: &World, title: &str) -> Uuid {
    let id = assignment_in(w, w.module, title, "text", "20").await;
    release(w, w.module, id).await;
    id
}

/// Publishes the module (not its other content) and the item.
async fn release(w: &World, module: Uuid, item: Uuid) {
    for path in [
        format!("/modules/{module}/publish"),
        format!("/items/{item}/publish"),
    ] {
        let body = path.contains("modules").then(|| json!({"cascade": false}));
        ok(&call(&w.http, "POST", Some(&w.owner.token), &path, body).await);
    }
}

async fn save(w: &World, token: &str, assignment: Uuid, student: Uuid, body: Value) -> Resp {
    call(
        &w.http,
        "PUT",
        Some(token),
        &format!("/assignments/{assignment}/grades/{student}"),
        Some(body),
    )
    .await
}

async fn grade(
    w: &World,
    assignment: Uuid,
    student: Uuid,
    score: Option<&str>,
    feedback: Option<&str>,
) -> Value {
    ok(&save(
        w,
        &w.owner.token,
        assignment,
        student,
        json!({"score": score, "feedback": feedback}),
    )
    .await)
}

async fn publish(w: &World, token: &str, assignment: Uuid, students: Value) -> Resp {
    call(
        &w.http,
        "POST",
        Some(token),
        &format!("/assignments/{assignment}/grades/publish"),
        Some(json!({"students": students})),
    )
    .await
}

async fn published(w: &World, assignment: Uuid, students: Value) -> Vec<Uuid> {
    let body = ok(&publish(w, &w.owner.token, assignment, students).await);
    body["published"]
        .as_array()
        .expect("published")
        .iter()
        .map(|id| Uuid::parse_str(id.as_str().expect("id")).expect("uuid"))
        .collect()
}

async fn my_grades(w: &World, token: &str) -> Value {
    ok(&call(
        &w.http,
        "GET",
        Some(token),
        &format!("/courses/{}/my-grades", w.course),
        None,
    )
    .await)
}

async fn gradebook(w: &World, token: &str, query: &str) -> Value {
    ok(&call(
        &w.http,
        "GET",
        Some(token),
        &format!("/courses/{}/gradebook{query}", w.course),
        None,
    )
    .await)
}

async fn set_name(w: &World, user: Uuid, name: &str, surname: &str) {
    sqlx::query!(
        "UPDATE auth.users SET name = $2, surname = $3 WHERE id = $1",
        user,
        name,
        surname
    )
    .execute(&w.pool)
    .await
    .unwrap();
}

async fn add_student(w: &World, email: &str, name: &str, surname: &str) -> Uuid {
    let student = user(&w.pool, &w.http, email, Role::Student).await;
    put_member(&w.http, &w.owner.token, w.course, student.id, "student").await;
    set_name(w, student.id, name, surname).await;
    student.id
}

async fn event_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM events.outbox"#)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn error_fields(response: &Resp) -> Vec<String> {
    response
        .problem(422)
        .errors
        .into_iter()
        .map(|e| e.field)
        .collect()
}

// --- the happy path ----------------------------------------------------------------------------

#[tokio::test]
async fn a_draft_is_invisible_to_the_student_until_it_is_published() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = open_assignment(&w, "Oral").await;

        let cell = grade(&w, a, w.student.id, Some("7.5"), Some("  Good answer  ")).await;
        assert_eq!(
            cell,
            json!({
                "draft_score": "7.50", "draft_feedback": "Good answer",
                "published_score": null, "published_feedback": null,
                "has_unpublished_changes": true,
            })
        );
        assert_eq!(
            event_count(&w.pool).await,
            0,
            "saving a draft emits no event"
        );
        assert_eq!(my_grades(&w, &w.student.token).await, json!({"items": []}));

        assert_eq!(published(&w, a, json!("all")).await, [w.student.id]);
        let mine = my_grades(&w, &w.student.token).await;
        assert_eq!(mine["items"].as_array().unwrap().len(), 1);
        let item = &mine["items"][0];
        assert_eq!(item["assignment_id"], json!(a));
        assert_eq!(item["title"], "Oral");
        assert_eq!(item["max_points"], "20.00");
        assert_eq!(item["is_bonus"], false);
        assert_eq!(item["score"], "7.50");
        assert_eq!(item["feedback"], "Good answer");
        assert!(item["published_at"].is_string());

        // Editing the draft changes only the draft: the student keeps the old grade.
        let edited = grade(&w, a, w.student.id, Some("9"), Some("Better")).await;
        assert_eq!(edited["draft_score"], "9.00");
        assert_eq!(edited["published_score"], "7.50");
        assert_eq!(edited["has_unpublished_changes"], true);
        assert_eq!(my_grades(&w, &w.student.token).await, mine);
        assert_eq!(event_count(&w.pool).await, 1);
    })
    .await;
}

#[tokio::test]
async fn a_student_never_receives_a_draft_value() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = open_assignment(&w, "Essay").await;
        grade(&w, a, w.student.id, Some("11.11"), Some("first")).await;
        published(&w, a, json!("all")).await;
        grade(&w, a, w.student.id, Some("17.77"), Some("DRAFT-ONLY-NOTE")).await;

        let response = call(
            &w.http,
            "GET",
            Some(&w.student.token),
            &format!("/courses/{}/my-grades", w.course),
            None,
        )
        .await;
        assert_eq!(response.status, 200);
        let raw = String::from_utf8_lossy(&response.body).into_owned();
        for secret in ["17.77", "DRAFT-ONLY-NOTE", "draft"] {
            assert!(
                !raw.contains(secret),
                "{secret} leaked to the student: {raw}"
            );
        }
        assert!(raw.contains("11.11"));

        // The staff gradebook is refused outright, and says nothing of the values.
        let refused = call(
            &w.http,
            "GET",
            Some(&w.student.token),
            &format!("/courses/{}/gradebook", w.course),
            None,
        )
        .await;
        let problem = refused.problem(403);
        let raw = String::from_utf8_lossy(&refused.body).into_owned();
        assert!(
            !raw.contains("17.77") && !raw.contains("DRAFT-ONLY-NOTE"),
            "{raw}"
        );
        assert!(problem.errors.is_empty());
    })
    .await;
}

#[tokio::test]
async fn a_no_submission_assignment_is_graded_like_any_other() {
    db_test(async |pool| {
        let w = world(pool).await;
        let oral = assignment_in(&w, w.module, "Oral answers", "no_submission", "12").await;
        release(&w, w.module, oral).await;

        grade(&w, oral, w.student.id, Some("10"), None).await;
        assert_eq!(published(&w, oral, json!("all")).await, [w.student.id]);
        let mine = my_grades(&w, &w.student.token).await;
        assert_eq!(mine["items"][0]["score"], "10.00");
        assert_eq!(mine["items"][0]["feedback"], Value::Null);
        let book = gradebook(&w, &w.owner.token, "").await;
        assert_eq!(book["columns"][0]["assignment_id"], json!(oral));
    })
    .await;
}

// --- values ------------------------------------------------------------------------------------

#[tokio::test]
async fn scores_run_from_zero_to_max_points_with_two_decimals() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = open_assignment(&w, "A").await;
        let s = w.student.id;

        for (sent, stored) in [
            ("0", "0.00"),
            ("0.00", "0.00"),
            ("20", "20.00"),
            ("19.99", "19.99"),
            ("0.01", "0.01"),
        ] {
            let cell = grade(&w, a, s, Some(sent), None).await;
            assert_eq!(cell["draft_score"], stored, "{sent}");
        }
        for bad in [
            json!("-0.01"),
            json!("20.01"),
            json!("1.005"),
            json!(5),
            json!(7.5),
            json!(true),
            json!(""),
            json!("abc"),
            json!(" 5"),
            json!("1e1"),
        ] {
            let response = save(
                &w,
                &w.owner.token,
                a,
                s,
                json!({"score": bad, "feedback": null}),
            )
            .await;
            let problem = response.problem(422);
            assert_eq!(problem.errors.len(), 1, "{bad}");
            assert_eq!(problem.errors[0].field, "score", "{bad}");
            assert_eq!(
                problem.errors[0].message,
                "must be a decimal string from 0 to 20.00 with at most two decimals",
                "{bad}"
            );
            let raw = String::from_utf8_lossy(&response.body).into_owned();
            assert!(
                !raw.contains("abc") && !raw.contains("1.005"),
                "the value is not echoed: {raw}"
            );
        }
        // Nothing was written by the refused requests: the last accepted one stands.
        let cell = grade(&w, a, s, None, None).await;
        assert_eq!(cell["draft_score"], Value::Null, "null clears the score");
    })
    .await;
}

#[tokio::test]
async fn feedback_is_trimmed_plain_text_of_at_most_four_thousand_characters() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = open_assignment(&w, "A").await;
        let s = w.student.id;

        assert_eq!(
            grade(&w, a, s, None, Some("  hi \n")).await["draft_feedback"],
            "hi"
        );
        assert_eq!(
            grade(&w, a, s, None, Some("   ")).await["draft_feedback"],
            Value::Null
        );
        assert_eq!(
            grade(&w, a, s, None, Some("")).await["draft_feedback"],
            Value::Null
        );
        let long = "ł".repeat(4000);
        assert_eq!(
            grade(&w, a, s, None, Some(&long)).await["draft_feedback"],
            json!(long)
        );

        for bad in [
            json!("ł".repeat(4001)),
            json!("a\u{0}b"),
            json!(5),
            json!(true),
            json!(["x"]),
            json!({"a": 1}),
        ] {
            let response = save(
                &w,
                &w.owner.token,
                a,
                s,
                json!({"score": null, "feedback": bad}),
            )
            .await;
            assert_eq!(error_fields(&response), ["feedback"], "{bad}");
        }
        // Both fields are named when both are wrong.
        let both = save(
            &w,
            &w.owner.token,
            a,
            s,
            json!({"score": "21", "feedback": 3}),
        )
        .await;
        assert_eq!(error_fields(&both), ["score", "feedback"]);
        // A PUT replaces the cell: both keys are required.
        for partial in [json!({}), json!({"score": "5"}), json!({"feedback": "x"})] {
            let response = save(&w, &w.owner.token, a, s, partial).await;
            assert_eq!(
                response.status,
                422,
                "{}",
                String::from_utf8_lossy(&response.body)
            );
        }
    })
    .await;
}

#[tokio::test]
async fn a_cell_with_only_feedback_is_published_and_shown() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = open_assignment(&w, "A").await;
        grade(&w, a, w.student.id, None, Some("Please resubmit")).await;
        assert_eq!(published(&w, a, json!("all")).await, [w.student.id]);
        let mine = my_grades(&w, &w.student.token).await;
        assert_eq!(mine["items"][0]["score"], Value::Null);
        assert_eq!(mine["items"][0]["feedback"], "Please resubmit");
        assert_eq!(event_count(&w.pool).await, 1);
    })
    .await;
}

// --- publishing --------------------------------------------------------------------------------

#[tokio::test]
async fn publishing_copies_only_what_differs_and_emits_one_event_per_changed_student() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = open_assignment(&w, "A").await;
        let second = add_student(&w, "second@x.edu", "Sam", "Second").await;
        let third = add_student(&w, "third@x.edu", "Tim", "Third").await;
        grade(&w, a, w.student.id, Some("5"), None).await;
        grade(&w, a, second, Some("6"), Some("ok")).await;
        // `third` has a draft equal to the (empty) published value: nothing to publish.
        grade(&w, a, third, None, None).await;

        let mut first_round = published(&w, a, json!("all")).await;
        first_round.sort();
        let mut expected = vec![w.student.id, second];
        expected.sort();
        assert_eq!(first_round, expected);
        assert_eq!(event_count(&w.pool).await, 2);

        assert!(
            published(&w, a, json!("all")).await.is_empty(),
            "nothing differs now"
        );
        assert_eq!(event_count(&w.pool).await, 2, "and nothing is emitted");

        // Only one draft changes; a list that names all three publishes just that one.
        grade(&w, a, second, Some("7"), Some("ok")).await;
        let again = published(&w, a, json!([w.student.id, second, third])).await;
        assert_eq!(again, [second]);
        assert_eq!(event_count(&w.pool).await, 3);

        // A listed set leaves the others' drafts alone.
        grade(&w, a, w.student.id, Some("8"), None).await;
        grade(&w, a, second, Some("9"), Some("ok")).await;
        assert_eq!(
            published(&w, a, json!([w.student.id])).await,
            [w.student.id]
        );
        let book = gradebook(&w, &w.owner.token, "").await;
        let cell_of = |who: Uuid| -> Value {
            book["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["user_id"] == json!(who))
                .unwrap()["cells"][0]
                .clone()
        };
        assert_eq!(cell_of(w.student.id)["has_unpublished_changes"], false);
        assert_eq!(cell_of(second)["published_score"], "7.00");
        assert_eq!(cell_of(second)["draft_score"], "9.00");
        assert_eq!(cell_of(second)["has_unpublished_changes"], true);

        // The events hold ids only.
        let events = sqlx::query!("SELECT type, version, payload FROM events.outbox")
            .fetch_all(&w.pool)
            .await
            .unwrap();
        assert_eq!(events.len(), 4);
        for event in events {
            assert_eq!(event.r#type, "grade.published");
            assert_eq!(event.version, 1);
            let mut keys: Vec<_> = event.payload.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            assert_eq!(keys, ["assignment_id", "course_id", "student_id"]);
            assert_eq!(event.payload["assignment_id"], json!(a));
            assert_eq!(event.payload["course_id"], json!(w.course));
        }
    })
    .await;
}

#[tokio::test]
async fn clearing_a_draft_and_publishing_it_retracts_the_grade() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = open_assignment(&w, "A").await;
        grade(&w, a, w.student.id, Some("12"), Some("fine")).await;
        published(&w, a, json!("all")).await;
        assert_eq!(
            my_grades(&w, &w.student.token).await["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(event_count(&w.pool).await, 1);

        let cleared = grade(&w, a, w.student.id, None, None).await;
        assert_eq!(
            cleared["has_unpublished_changes"], true,
            "the student still sees the old grade"
        );
        assert_eq!(
            my_grades(&w, &w.student.token).await["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        assert_eq!(
            published(&w, a, json!("all")).await,
            [w.student.id],
            "a retraction is a copy"
        );
        assert_eq!(my_grades(&w, &w.student.token).await, json!({"items": []}));
        let cell = &gradebook(&w, &w.owner.token, "").await["items"][0]["cells"][0];
        assert_eq!(cell["published_score"], Value::Null);
        assert_eq!(cell["published_feedback"], Value::Null);
        assert_eq!(cell["has_unpublished_changes"], false);
        assert_eq!(event_count(&w.pool).await, 1, "a retraction emits no event");
    })
    .await;
}

#[tokio::test]
async fn publish_needs_a_body_and_checks_the_listed_students() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = open_assignment(&w, "A").await;
        grade(&w, a, w.student.id, Some("5"), None).await;
        let path = format!("/assignments/{a}/grades/publish");
        let token = Some(w.owner.token.as_str());

        // The body is required and has no default.
        let none = call(&w.http, "POST", token, &path, None).await;
        assert!(none.status == 415 || none.status == 422, "{}", none.status);
        for bad in [
            json!({}),
            json!({"students": null}),
            json!({"students": "everyone"}),
            json!({"students": 1}),
            json!({"students": ["x"]}),
        ] {
            let response = call(&w.http, "POST", token, &path, Some(bad.clone())).await;
            assert_eq!(response.status, 422, "{bad}");
        }
        // A listed student must be a current student of the course; each offender is named by its
        // position in the list.
        for outside in [w.outsider.id, w.owner.id, w.assistant.id, Uuid::now_v7()] {
            let response = publish(&w, &w.owner.token, a, json!([w.student.id, outside])).await;
            assert_eq!(error_fields(&response), ["students[1]"], "{outside}");
        }
        let two = publish(
            &w,
            &w.owner.token,
            a,
            json!([w.outsider.id, w.student.id, w.owner.id]),
        )
        .await;
        let problem = two.problem(422);
        let fields: Vec<_> = problem.errors.iter().map(|e| e.field.as_str()).collect();
        assert_eq!(fields, ["students[0]", "students[2]"]);
        assert_eq!(
            problem.errors[0].message,
            "is not a current student of the course"
        );
        assert_eq!(
            event_count(&w.pool).await,
            0,
            "a refused publish changed nothing"
        );
        assert_eq!(my_grades(&w, &w.student.token).await, json!({"items": []}));
        // Duplicates are fine, and a student with no grade row is not an error.
        let blank = add_student(&w, "blank@x.edu", "Bo", "Blank").await;
        let ok = published(&w, a, json!([w.student.id, w.student.id, blank])).await;
        assert_eq!(ok, [w.student.id]);
        assert!(published(&w, a, json!([])).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn every_save_and_publish_is_kept_in_the_history() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = open_assignment(&w, "A").await;
        grade(&w, a, w.student.id, Some("5"), Some("one")).await;
        grade(&w, a, w.student.id, Some("6"), None).await;
        published(&w, a, json!("all")).await;
        grade(&w, a, w.student.id, None, None).await;
        published(&w, a, json!("all")).await;
        // A publish that changes nothing adds nothing.
        published(&w, a, json!("all")).await;

        let rows = sqlx::query!(
            r#"SELECT action, actor_id, score::text AS score, feedback
               FROM gradebook.grade_history WHERE assignment_id = $1 AND student_id = $2
               ORDER BY id"#,
            a,
            w.student.id
        )
        .fetch_all(&w.pool)
        .await
        .unwrap();
        let seen: Vec<_> = rows
            .iter()
            .map(|r| (r.action.as_str(), r.score.as_deref(), r.feedback.as_deref()))
            .collect();
        assert_eq!(
            seen,
            [
                ("save_draft", Some("5.00"), Some("one")),
                ("save_draft", Some("6.00"), None),
                ("publish", Some("6.00"), None),
                ("save_draft", None, None),
                ("publish", None, None),
            ]
        );
        assert!(rows.iter().all(|r| r.actor_id == Some(w.owner.id)));
    })
    .await;
}

// --- who sees what -----------------------------------------------------------------------------

#[tokio::test]
async fn a_student_who_is_not_in_the_course_has_no_cell() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = open_assignment(&w, "A").await;
        for who in [w.outsider.id, w.owner.id, w.assistant.id, Uuid::now_v7()] {
            let response = save(
                &w,
                &w.owner.token,
                a,
                who,
                json!({"score": "5", "feedback": null}),
            )
            .await;
            response.problem(404);
        }
        let count = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM gradebook.grades"#)
            .fetch_one(&w.pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
        save(
            &w,
            &w.owner.token,
            Uuid::now_v7(),
            w.student.id,
            json!({"score": null, "feedback": null}),
        )
        .await
        .problem(404);
    })
    .await;
}

#[tokio::test]
async fn the_gradebook_lists_current_and_former_students_sorted_and_paged() {
    db_test(async |pool| {
        let w = world(pool).await;
        set_name(&w, w.student.id, "Anna", "Adams").await;
        let zed = add_student(&w, "zed@x.edu", "Zed", "Brown").await;
        let amy = add_student(&w, "amy@x.edu", "Amy", "Brown").await;
        let cy = add_student(&w, "cy@x.edu", "Cy", "Young").await;
        let left = add_student(&w, "left@x.edu", "Lee", "Aaron").await;

        // Columns in tree order: M's items, then its sub-module's, then the next module's. Drafts count.
        let sub = module(&w, "Sub", Some(w.module)).await;
        let next = module(&w, "Next", None).await;
        let first = assignment_in(&w, w.module, "first", "text", "10").await;
        let in_sub = assignment_in(&w, sub, "in-sub", "no_submission", "5").await;
        let last = assignment_in(&w, next, "last", "text", "10").await;
        let second = assignment_in(&w, w.module, "second", "text", "10").await;
        release(&w, w.module, first).await;

        for who in [w.student.id, left] {
            ok(&save(
                &w,
                &w.owner.token,
                first,
                who,
                json!({"score": "3", "feedback": null}),
            )
            .await);
        }
        ok(&save(
            &w,
            &w.owner.token,
            last,
            cy,
            json!({"score": "4", "feedback": "x"}),
        )
        .await);
        // `left` leaves the course: off the roster, but the grade stays and the gradebook still
        // lists the person, as a former student (ADR-0037).
        let removed = call(
            &w.http,
            "PUT",
            Some(&w.owner.token),
            &format!("/courses/{}/members/{left}", w.course),
            Some(json!({"role": "assistant"})),
        )
        .await;
        assert_eq!(removed.status, 200);
        let kept = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM gradebook.grades WHERE student_id = $1"#,
            left
        )
        .fetch_one(&w.pool)
        .await
        .unwrap();
        assert_eq!(kept, 1);

        let page = gradebook(&w, &w.owner.token, "?limit=2").await;
        let titles: Vec<_> = page["columns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["title"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(titles, ["first", "second", "in-sub", "last"]);
        assert_eq!(page["columns"][0]["status"], "published");
        assert_eq!(page["columns"][1]["status"], "draft");
        assert_eq!(page["columns"][2]["max_points"], "5.00");
        assert_eq!(page["columns"][0]["is_bonus"], false);

        let mut order = Vec::new();
        let mut cursor = None::<String>;
        let mut pages = 0;
        let mut first_page = Some(page);
        loop {
            let page = match first_page.take() {
                Some(page) => page,
                None => {
                    gradebook(
                        &w,
                        &w.owner.token,
                        &format!("?limit=2&cursor={}", cursor.clone().unwrap()),
                    )
                    .await
                }
            };
            pages += 1;
            assert_eq!(
                page["columns"].as_array().unwrap().len(),
                4,
                "columns are on every page"
            );
            for row in page["items"].as_array().unwrap() {
                order.push((
                    row["surname"].as_str().unwrap().to_owned(),
                    row["name"].as_str().unwrap().to_owned(),
                    row["user_id"].clone(),
                ));
            }
            match page["next_cursor"].as_str() {
                Some(next) => cursor = Some(next.to_owned()),
                None => break,
            }
        }
        assert_eq!(pages, 3);
        let names: Vec<_> = order.iter().map(|(s, n, _)| format!("{s} {n}")).collect();
        assert_eq!(
            names,
            [
                "Aaron Lee",
                "Adams Anna",
                "Brown Amy",
                "Brown Zed",
                "Young Cy"
            ],
            "surname, name; `left` is listed because the grade is kept"
        );
        assert_eq!(order[0].2, json!(left));
        let all_rows = gradebook(&w, &w.owner.token, "").await;
        let former: Vec<_> = all_rows["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["former"] == json!(true))
            .map(|row| row["user_id"].clone())
            .collect();
        assert_eq!(former, [json!(left)], "only `left` is a former student");
        assert_eq!(order[2].2, json!(amy));
        assert_eq!(order[3].2, json!(zed));

        // A student appears with cells only for what was graded.
        let all = gradebook(&w, &w.owner.token, "").await;
        let cells_of = |who: Uuid| {
            all["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["user_id"] == json!(who))
                .unwrap()["cells"]
                .clone()
        };
        assert_eq!(cells_of(w.student.id).as_array().unwrap().len(), 1);
        assert_eq!(cells_of(w.student.id)[0]["assignment_id"], json!(first));
        assert_eq!(cells_of(cy)[0]["assignment_id"], json!(last));
        assert_eq!(cells_of(cy)[0]["draft_score"], "4.00");
        assert_eq!(cells_of(amy), json!([]));
        assert_eq!(all["next_cursor"], Value::Null);
        let _ = (second, in_sub);

        // Bad cursors and limits are 422.
        for query in ["?cursor=nope", "?limit=0", "?limit=101"] {
            let response = call(
                &w.http,
                "GET",
                Some(&w.owner.token),
                &format!("/courses/{}/gradebook{query}", w.course),
                None,
            )
            .await;
            response.problem(422);
        }
    })
    .await;
}

#[tokio::test]
async fn my_grades_leaves_out_assignments_the_student_may_not_see() {
    db_test(async |pool| {
        let w = world(pool).await;
        let open = open_assignment(&w, "open").await;
        let draft_item = assignment_in(&w, w.module, "draft item", "text", "5").await;
        let hidden_module = module(&w, "Hidden", None).await;
        let in_hidden = assignment_in(&w, hidden_module, "in hidden module", "text", "5").await;
        release(&w, hidden_module, in_hidden).await;
        let scheduled = assignment_in(&w, w.module, "scheduled", "text", "5").await;
        release(&w, w.module, scheduled).await;
        for a in [open, draft_item, in_hidden, scheduled] {
            grade(&w, a, w.student.id, Some("1"), None).await;
            published(&w, a, json!("all")).await;
        }
        // The hidden module goes back to draft, and one item is scheduled for later.
        let back = call(
            &w.http,
            "PATCH",
            Some(&w.owner.token),
            &format!("/items/{scheduled}"),
            Some(json!({"publish_at": FUTURE})),
        )
        .await;
        assert_eq!(back.status, 200);
        sqlx::query!(
            "UPDATE content.modules SET status = 'draft' WHERE id = $1",
            hidden_module
        )
        .execute(&w.pool)
        .await
        .unwrap();

        let mine = my_grades(&w, &w.student.token).await;
        let titles: Vec<_> = mine["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["title"].as_str().unwrap())
            .collect();
        assert_eq!(titles, ["open"]);
        // Staff still see every column.
        let book = gradebook(&w, &w.owner.token, "").await;
        assert_eq!(book["columns"].as_array().unwrap().len(), 4);

        // Once released, they appear, in tree order.
        release(&w, w.module, draft_item).await;
        let titles_now: Vec<String> = my_grades(&w, &w.student.token).await["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["title"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(titles_now, ["open", "draft item"]);
    })
    .await;
}

// --- authorization -----------------------------------------------------------------------------

#[tokio::test]
async fn who_may_grade_publish_and_read() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = open_assignment(&w, "A").await;
        let hidden = assignment_in(&w, w.module, "draft", "text", "5").await;
        let s = w.student.id;
        let cell = json!({"score": "5", "feedback": null});

        struct Who<'a>(&'static str, Option<&'a str>);
        let anonymous = Who("anonymous", None);
        let owner = Who("owner", Some(&w.owner.token));
        let assistant = Who("assistant", Some(&w.assistant.token));
        let admin = Who("admin", Some(&w.admin.token));
        let student = Who("student", Some(&w.student.token));
        let stranger = Who("other course's teacher", Some(&w.stranger.token));
        let outsider = Who("a student of no course", Some(&w.outsider.token));

        let put = format!("/assignments/{a}/grades/{s}");
        let post = format!("/assignments/{a}/grades/publish");
        let book = format!("/courses/{}/gradebook", w.course);
        let mine = format!("/courses/{}/my-grades", w.course);
        let all = json!({"students": "all"});

        for (who, expected) in [
            (&owner, 200),
            (&assistant, 200),
            (&admin, 200),
            (&student, 403),
            (&stranger, 404),
            (&outsider, 404),
            (&anonymous, 401),
        ] {
            let response = call(&w.http, "PUT", who.1, &put, Some(cell.clone())).await;
            assert_eq!(response.status, expected, "save as {}", who.0);
            let response = call(&w.http, "POST", who.1, &post, Some(all.clone())).await;
            assert_eq!(response.status, expected, "publish as {}", who.0);
            let response = call(&w.http, "GET", who.1, &book, None).await;
            assert_eq!(response.status, expected, "gradebook as {}", who.0);
        }
        for (who, expected) in [
            (&student, 200),
            (&owner, 403),
            (&assistant, 403),
            (&admin, 403),
            (&stranger, 404),
            (&outsider, 404),
            (&anonymous, 401),
        ] {
            let response = call(&w.http, "GET", who.1, &mine, None).await;
            assert_eq!(response.status, expected, "my-grades as {}", who.0);
        }
        // An assignment a student may not see is a 404 for them, not a 403.
        let put_hidden = format!("/assignments/{hidden}/grades/{s}");
        let post_hidden = format!("/assignments/{hidden}/grades/publish");
        for (who, expected) in [(&owner, 200), (&student, 404), (&stranger, 404)] {
            assert_eq!(
                call(&w.http, "PUT", who.1, &put_hidden, Some(cell.clone()))
                    .await
                    .status,
                expected,
                "{}",
                who.0
            );
            assert_eq!(
                call(&w.http, "POST", who.1, &post_hidden, Some(all.clone()))
                    .await
                    .status,
                expected,
                "{}",
                who.0
            );
        }
        // The error bodies are Problem Details.
        call(&w.http, "PUT", Some(&w.student.token), &put, Some(cell))
            .await
            .problem(403);
        call(&w.http, "GET", None, &book, None).await.problem(401);
    })
    .await;
}

#[tokio::test]
async fn a_draft_course_hides_the_gradebook_from_its_students() {
    db_test(async |pool| {
        let w = world(pool).await;
        sqlx::query!(
            "UPDATE teaching.courses SET status = 'draft' WHERE id = $1",
            w.course
        )
        .execute(&w.pool)
        .await
        .unwrap();
        for path in [
            format!("/courses/{}/gradebook", w.course),
            format!("/courses/{}/my-grades", w.course),
        ] {
            call(&w.http, "GET", Some(&w.student.token), &path, None)
                .await
                .problem(404);
        }
        // Staff still reach the gradebook of a draft course.
        gradebook(&w, &w.owner.token, "").await;
    })
    .await;
}

#[tokio::test]
async fn admin_grade_writes_are_audited_and_staff_writes_and_reads_are_not() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = open_assignment(&w, "A").await;
        let audited = |pool: &PgPool| {
            let pool = pool.clone();
            async move {
                sqlx::query!(
                    "SELECT actor_id, action, resource_type, resource_id, meta FROM events.audit_log
                     WHERE action LIKE 'grade.%' ORDER BY id"
                )
                .fetch_all(&pool)
                .await
                .unwrap()
            }
        };

        // Staff writes and every read write nothing.
        ok(&save(
            &w,
            &w.owner.token,
            a,
            w.student.id,
            json!({"score": "5", "feedback": null}),
        )
        .await);
        ok(&save(
            &w,
            &w.assistant.token,
            a,
            w.student.id,
            json!({"score": "6", "feedback": null}),
        )
        .await);
        published(&w, a, json!("all")).await;
        gradebook(&w, &w.admin.token, "").await;
        assert!(audited(&w.pool).await.is_empty());

        ok(&save(
            &w,
            &w.admin.token,
            a,
            w.student.id,
            json!({"score": "7", "feedback": null}),
        )
        .await);
        // A publish that changes nothing is not audited; one that does is.
        assert!(
            ok(&publish(&w, &w.admin.token, a, json!("all")).await)["published"]
                .as_array()
                .unwrap()
                .len()
                == 1
        );
        assert!(
            ok(&publish(&w, &w.admin.token, a, json!("all")).await)["published"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        // A refused admin write is not audited either.
        save(
            &w,
            &w.admin.token,
            a,
            w.student.id,
            json!({"score": "99", "feedback": null}),
        )
        .await
        .problem(422);

        let rows = audited(&w.pool).await;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].action, "grade.save");
        assert_eq!(rows[0].actor_id, Some(w.admin.id));
        assert_eq!(rows[0].resource_type, "assignment");
        assert_eq!(rows[0].resource_id, a);
        assert_eq!(
            rows[0].meta,
            json!({"course_id": w.course, "student_id": w.student.id})
        );
        assert_eq!(rows[1].action, "grade.publish");
        assert_eq!(
            rows[1].meta,
            json!({"course_id": w.course, "student_ids": [w.student.id]})
        );
        let raw = format!(
            "{:?}",
            rows.iter().map(|r| r.meta.to_string()).collect::<Vec<_>>()
        );
        assert!(!raw.contains("7.00"), "no grade in the audit log");
    })
    .await;
}

#[tokio::test]
async fn the_gradebook_endpoints_are_in_the_openapi_document() {
    db_test(async |pool| {
        let w = world(pool).await;
        let spec = ok(&call(&w.http, "GET", None, "/openapi.json", None).await);
        for (path, method) in [
            ("/api/v1/assignments/{id}/grades/{student_id}", "put"),
            ("/api/v1/assignments/{id}/grades/publish", "post"),
            ("/api/v1/courses/{id}/gradebook", "get"),
            ("/api/v1/courses/{id}/my-grades", "get"),
        ] {
            assert!(spec["paths"][path][method].is_object(), "{method} {path}");
        }
        for schema in [
            "SaveGradeRequest",
            "PublishGradesRequest",
            "GradeCell",
            "GradebookPage",
            "MyGrades",
        ] {
            assert!(
                spec["components"]["schemas"][schema].is_object(),
                "{schema}"
            );
        }
        let schemas = &spec["components"]["schemas"];
        let cells = schemas["GradebookRow"]["properties"]["cells"]["description"]
            .as_str()
            .unwrap();
        assert!(
            cells.contains("assignment_id") && cells.contains("no grade"),
            "{cells}"
        );
        // The flattened cell is `allOf` the grade fields and the key.
        let own = &schemas["GradebookCell"]["allOf"][1];
        let key = own["properties"]["assignment_id"]["description"]
            .as_str()
            .unwrap();
        assert!(key.contains("map"), "{key}");
        assert!(
            own["required"]
                .as_array()
                .unwrap()
                .contains(&json!("assignment_id"))
        );
        let my = spec["components"]["schemas"]["MyGrade"]["properties"]
            .as_object()
            .unwrap();
        assert!(
            my.keys().all(|key| !key.starts_with("draft")),
            "the student schema has no draft field"
        );
    })
    .await;
}

/// A grade row whose values are all empty, with two history rows behind it.
async fn saved_and_cleared(w: &World, assignment: Uuid) {
    grade(w, assignment, w.student.id, Some("3"), Some("x")).await;
    grade(w, assignment, w.student.id, None, None).await;
}

// --- the callbacks `content` calls --------------------------------------------------------------

use server::learning::gradebook::{self, GradebookError};

async fn rows_of(pool: &PgPool, assignment: Uuid) -> (i64, i64) {
    let grades = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM gradebook.grades WHERE assignment_id = $1"#,
        assignment
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let history = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM gradebook.grade_history WHERE assignment_id = $1"#,
        assignment
    )
    .fetch_one(pool)
    .await
    .unwrap();
    (grades, history)
}

#[tokio::test]
async fn max_score_is_the_highest_draft_or_published_score() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = open_assignment(&w, "A").await;
        let other = add_student(&w, "other@x.edu", "Oto", "Other").await;

        assert_eq!(
            gradebook::max_score(&mut w.pool.acquire().await.unwrap(), a)
                .await
                .unwrap(),
            None,
            "no grade row"
        );
        grade(&w, a, w.student.id, None, Some("only words")).await;
        assert_eq!(
            gradebook::max_score(&mut w.pool.acquire().await.unwrap(), a)
                .await
                .unwrap(),
            None,
            "no score yet"
        );

        grade(&w, a, w.student.id, Some("8"), None).await;
        published(&w, a, json!("all")).await;
        grade(&w, a, w.student.id, Some("5"), None).await;
        let max = gradebook::max_score(&mut w.pool.acquire().await.unwrap(), a)
            .await
            .unwrap();
        assert_eq!(
            max.map(|m| m.to_string()),
            Some("8.00".to_owned()),
            "the published one is higher"
        );

        grade(&w, a, other, Some("12.5"), None).await;
        let max = gradebook::max_score(&mut w.pool.acquire().await.unwrap(), a)
            .await
            .unwrap();
        assert_eq!(
            max.map(|m| m.to_string()),
            Some("12.50".to_owned()),
            "a draft counts"
        );

        // Another assignment's scores are not mixed in, and the history is not read.
        let b = open_assignment(&w, "B").await;
        assert_eq!(
            gradebook::max_score(&mut w.pool.acquire().await.unwrap(), b)
                .await
                .unwrap(),
            None
        );
        grade(&w, a, other, None, None).await;
        grade(&w, a, w.student.id, None, None).await;
        published(&w, a, json!("all")).await;
        assert_eq!(
            gradebook::max_score(&mut w.pool.acquire().await.unwrap(), a)
                .await
                .unwrap(),
            None,
            "cleared and retracted"
        );
    })
    .await;
}

#[tokio::test]
async fn has_grades_ignores_rows_whose_values_are_all_empty() {
    db_test(async |pool| {
        let w = world(pool).await;
        let sub = module(&w, "Sub", Some(w.module)).await;
        let elsewhere = module(&w, "Elsewhere", None).await;
        let a = assignment_in(&w, w.module, "A", "text", "10").await;
        let in_sub = assignment_in(&w, sub, "S", "text", "10").await;
        let other = assignment_in(&w, elsewhere, "O", "text", "10").await;

        async fn flags(w: &World, sub: Uuid, ids: [Uuid; 3]) -> [bool; 6] {
            let [a, in_sub, other] = ids;
            // A connection for the length of the six reads only: a test must never hold a pooled
            // connection across a request, or it waits for the server's own connection (A-10).
            let conn = &mut w.pool.acquire().await.unwrap();
            [
                gradebook::has_grades_for_assignment(conn, a).await.unwrap(),
                gradebook::has_grades_for_assignment(conn, in_sub)
                    .await
                    .unwrap(),
                gradebook::has_grades_for_assignment(conn, other)
                    .await
                    .unwrap(),
                gradebook::has_grades_in_module(conn, w.module)
                    .await
                    .unwrap(),
                gradebook::has_grades_in_module(conn, sub).await.unwrap(),
                gradebook::has_grades_in_course(conn, w.course)
                    .await
                    .unwrap(),
            ]
        }
        let ids = [a, in_sub, other];
        assert_eq!(flags(&w, sub, ids).await, [false; 6]);

        // An empty row (a grade saved and cleared) is not a grade.
        grade(&w, in_sub, w.student.id, Some("3"), None).await;
        grade(&w, in_sub, w.student.id, None, None).await;
        assert_eq!(rows_of(&w.pool, in_sub).await.0, 1);
        assert_eq!(flags(&w, sub, ids).await, [false; 6]);

        // A value in the sub-module counts for the sub-module, its parent and the course only.
        grade(&w, in_sub, w.student.id, None, Some("words")).await;
        assert_eq!(
            flags(&w, sub, ids).await,
            [false, true, false, true, true, true]
        );
        // A published value counts even when the draft was cleared.
        grade(&w, a, w.student.id, Some("1"), None).await;
        published(&w, a, json!("all")).await;
        grade(&w, a, w.student.id, None, None).await;
        assert_eq!(
            flags(&w, sub, ids).await,
            [true, true, false, true, true, true]
        );
        grade(&w, other, w.student.id, Some("2"), None).await;
        assert_eq!(
            flags(&w, sub, ids).await,
            [true, true, true, true, true, true]
        );
    })
    .await;
}

#[tokio::test]
async fn a_purge_removes_empty_rows_and_their_history_and_refuses_a_graded_scope() {
    db_test(async |pool| {
        let w = world(pool).await;
        let sub = module(&w, "Sub", Some(w.module)).await;
        let elsewhere = module(&w, "Elsewhere", None).await;
        let a = assignment_in(&w, w.module, "A", "text", "10").await;
        let in_sub = assignment_in(&w, sub, "S", "text", "10").await;
        let other = assignment_in(&w, elsewhere, "O", "text", "10").await;

        for id in [a, in_sub, other] {
            saved_and_cleared(&w, id).await;
            assert_eq!(rows_of(&w.pool, id).await, (1, 2));
        }

        // One assignment: its row and its history go, the others stay.
        assert_eq!(
            gradebook::purge_assignment(&mut w.pool.acquire().await.unwrap(), a)
                .await
                .unwrap(),
            1
        );
        assert_eq!(rows_of(&w.pool, a).await, (0, 0));
        assert_eq!(rows_of(&w.pool, in_sub).await, (1, 2));

        // A module takes its sub-modules with it, and nothing of another module.
        assert_eq!(
            gradebook::purge_module(&mut w.pool.acquire().await.unwrap(), w.module)
                .await
                .unwrap(),
            1
        );
        assert_eq!(rows_of(&w.pool, in_sub).await, (0, 0));
        assert_eq!(rows_of(&w.pool, other).await, (1, 2));

        // A graded scope is refused and nothing is deleted.
        grade(&w, other, w.student.id, Some("4"), None).await;
        grade(&w, a, w.student.id, Some("1"), None).await;
        let before = (rows_of(&w.pool, a).await, rows_of(&w.pool, other).await);
        for result in [
            gradebook::purge_assignment(&mut w.pool.acquire().await.unwrap(), other).await,
            gradebook::purge_module(&mut w.pool.acquire().await.unwrap(), elsewhere).await,
            gradebook::purge_course(&mut w.pool.acquire().await.unwrap(), w.course).await,
        ] {
            assert!(
                matches!(result, Err(GradebookError::HasGrades)),
                "{result:?}"
            );
        }
        assert_eq!(
            (rows_of(&w.pool, a).await, rows_of(&w.pool, other).await),
            before
        );

        // Once every value is cleared, the course purge takes the rest.
        grade(&w, other, w.student.id, None, None).await;
        grade(&w, a, w.student.id, None, None).await;
        assert_eq!(
            gradebook::purge_course(&mut w.pool.acquire().await.unwrap(), w.course)
                .await
                .unwrap(),
            2
        );
        assert_eq!(rows_of(&w.pool, a).await, (0, 0));
        assert_eq!(rows_of(&w.pool, other).await, (0, 0));
    })
    .await;
}

// --- the guards content owes the gradebook ---------------------------------------------------------

async fn set_points(w: &World, item: Uuid, points: &str) -> Resp {
    call(
        &w.http,
        "PATCH",
        Some(&w.owner.token),
        &format!("/items/{item}"),
        Some(json!({"max_points": points})),
    )
    .await
}

async fn delete(w: &World, path: &str) -> Resp {
    call(&w.http, "DELETE", Some(&w.owner.token), path, None).await
}

async fn max_points_of(w: &World, item: Uuid) -> String {
    let item = ok(&call(
        &w.http,
        "GET",
        Some(&w.owner.token),
        &format!("/items/{item}"),
        None,
    )
    .await);
    item["assignment"]["max_points"]
        .as_str()
        .unwrap()
        .to_owned()
}

const LOWERING_MESSAGE: &str = "max_points cannot be lowered below a score that was already given";
const DELETE_MESSAGE: &str = "it has grades: clear them before deleting";

fn conflict(response: &Resp, message: &str) {
    let problem = response.problem(409);
    assert_eq!(problem.detail.as_deref(), Some(message));
}

#[tokio::test]
async fn max_points_cannot_drop_below_a_draft_or_published_score() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = open_assignment(&w, "A").await;
        let other = add_student(&w, "other@x.edu", "Oto", "Other").await;

        // Nothing graded yet, and an empty row: lowering is free.
        assert_eq!(
            ok(&set_points(&w, a, "15").await)["assignment"]["max_points"],
            "15.00"
        );
        grade(&w, a, other, Some("14"), None).await;
        grade(&w, a, other, None, None).await;
        assert_eq!(
            ok(&set_points(&w, a, "10").await)["assignment"]["max_points"],
            "10.00"
        );

        // A draft score blocks, down to exactly the score.
        grade(&w, a, w.student.id, Some("8"), None).await;
        conflict(&set_points(&w, a, "7.99").await, LOWERING_MESSAGE);
        assert_eq!(max_points_of(&w, a).await, "10.00");
        assert_eq!(
            ok(&set_points(&w, a, "8").await)["assignment"]["max_points"],
            "8.00"
        );
        assert_eq!(
            ok(&set_points(&w, a, "8.00").await)["assignment"]["max_points"],
            "8.00",
            "no change"
        );
        assert_eq!(
            ok(&set_points(&w, a, "20").await)["assignment"]["max_points"],
            "20.00",
            "raising"
        );

        // A published score blocks even after the draft was cleared; a higher one anywhere counts.
        grade(&w, a, other, Some("12"), None).await;
        published(&w, a, json!("all")).await;
        grade(&w, a, other, None, None).await;
        conflict(&set_points(&w, a, "11").await, LOWERING_MESSAGE);
        assert_eq!(
            ok(&set_points(&w, a, "12").await)["assignment"]["max_points"],
            "12.00"
        );
        // Only the scores count, not feedback.
        grade(&w, a, other, None, Some("words")).await;
        published(&w, a, json!("all")).await;
        grade(&w, a, other, None, None).await;
        published(&w, a, json!("all")).await;
        grade(&w, a, w.student.id, None, None).await;
        published(&w, a, json!("all")).await;
        assert_eq!(
            ok(&set_points(&w, a, "1").await)["assignment"]["max_points"],
            "1.00"
        );
    })
    .await;
}

#[tokio::test]
async fn an_assignment_with_a_grade_cannot_be_deleted_and_an_empty_one_is_purged() {
    db_test(async |pool| {
        let w = world(pool).await;
        let graded = open_assignment(&w, "graded").await;
        let empty = open_assignment(&w, "empty").await;
        let plain = assignment_in(&w, w.module, "plain", "no_submission", "5").await;
        let text = {
            let response = call(
                &w.http,
                "POST",
                Some(&w.owner.token),
                &format!("/modules/{}/items", w.module),
                Some(json!({"kind": "text", "title": "T"})),
            )
            .await;
            id_of(&response.json())
        };

        // A draft value blocks; nothing is deleted and the grade is intact.
        grade(&w, graded, w.student.id, Some("3"), None).await;
        conflict(
            &delete(&w, &format!("/items/{graded}")).await,
            DELETE_MESSAGE,
        );
        assert_eq!(rows_of(&w.pool, graded).await, (1, 1));
        assert_eq!(
            max_points_of(&w, graded).await,
            "20.00",
            "the item is still there"
        );
        // A published value blocks after the draft is cleared, feedback alone blocks too.
        published(&w, graded, json!("all")).await;
        grade(&w, graded, w.student.id, None, None).await;
        conflict(
            &delete(&w, &format!("/items/{graded}")).await,
            DELETE_MESSAGE,
        );
        published(&w, graded, json!("all")).await;
        grade(&w, graded, w.student.id, None, Some("only words")).await;
        conflict(
            &delete(&w, &format!("/items/{graded}")).await,
            DELETE_MESSAGE,
        );

        // A row with nothing in it does not block, and goes with its history.
        saved_and_cleared(&w, empty).await;
        assert_eq!(rows_of(&w.pool, empty).await, (1, 2));
        assert_eq!(delete(&w, &format!("/items/{empty}")).await.status, 204);
        assert_eq!(rows_of(&w.pool, empty).await, (0, 0));
        call(
            &w.http,
            "GET",
            Some(&w.owner.token),
            &format!("/items/{empty}"),
            None,
        )
        .await
        .problem(404);
        // The graded neighbour is untouched.
        assert_eq!(rows_of(&w.pool, graded).await.0, 1);

        // No grade row at all, and an item that is not an assignment.
        assert_eq!(delete(&w, &format!("/items/{plain}")).await.status, 204);
        assert_eq!(delete(&w, &format!("/items/{text}")).await.status, 204);

        // Clearing the last value lets the graded one go too.
        grade(&w, graded, w.student.id, None, None).await;
        published(&w, graded, json!("all")).await;
        assert_eq!(delete(&w, &format!("/items/{graded}")).await.status, 204);
        assert_eq!(rows_of(&w.pool, graded).await, (0, 0));
    })
    .await;
}

#[tokio::test]
async fn a_module_with_a_grade_below_it_cannot_be_deleted_and_an_empty_one_is_purged() {
    db_test(async |pool| {
        let w = world(pool).await;
        let top = module(&w, "Top", None).await;
        let sub = module(&w, "Sub", Some(top)).await;
        let in_top = assignment_in(&w, top, "in top", "text", "5").await;
        let in_sub = assignment_in(&w, sub, "in sub", "no_submission", "5").await;
        let elsewhere = assignment_in(&w, w.module, "elsewhere", "text", "5").await;
        grade(&w, elsewhere, w.student.id, Some("1"), None).await;

        // A grade in the sub-module blocks the sub-module and its parent, not the other module.
        grade(&w, in_sub, w.student.id, Some("2"), None).await;
        conflict(
            &delete(&w, &format!("/modules/{sub}")).await,
            DELETE_MESSAGE,
        );
        conflict(
            &delete(&w, &format!("/modules/{top}")).await,
            DELETE_MESSAGE,
        );
        assert_eq!(rows_of(&w.pool, in_sub).await, (1, 1));

        // Only empty rows left: the parent goes with the sub-module and every row and history.
        grade(&w, in_sub, w.student.id, None, None).await;
        saved_and_cleared(&w, in_top).await;
        assert_eq!(delete(&w, &format!("/modules/{top}")).await.status, 204);
        assert_eq!(rows_of(&w.pool, in_top).await, (0, 0));
        assert_eq!(rows_of(&w.pool, in_sub).await, (0, 0));
        assert_eq!(
            rows_of(&w.pool, elsewhere).await,
            (1, 1),
            "another module is untouched"
        );
        conflict(
            &delete(&w, &format!("/modules/{}", w.module)).await,
            DELETE_MESSAGE,
        );
        // A module with no assignment at all is deleted as before.
        let bare = module(&w, "Bare", None).await;
        assert_eq!(delete(&w, &format!("/modules/{bare}")).await.status, 204);
    })
    .await;
}

#[tokio::test]
async fn an_admin_delete_is_audited_once_and_a_refused_one_not_at_all() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = open_assignment(&w, "A").await;
        saved_and_cleared(&w, a).await;
        let audit_count = |action: &'static str| {
            let pool = w.pool.clone();
            async move {
                sqlx::query_scalar!(
                    r#"SELECT count(*) AS "n!" FROM events.audit_log WHERE action = $1"#,
                    action
                )
                .fetch_one(&pool)
                .await
                .unwrap()
            }
        };
        grade(&w, a, w.student.id, Some("1"), None).await;
        let refused = call(
            &w.http,
            "DELETE",
            Some(&w.admin.token),
            &format!("/items/{a}"),
            None,
        )
        .await;
        conflict(&refused, DELETE_MESSAGE);
        assert_eq!(audit_count("content.item.delete").await, 0);
        grade(&w, a, w.student.id, None, None).await;
        let done = call(
            &w.http,
            "DELETE",
            Some(&w.admin.token),
            &format!("/items/{a}"),
            None,
        )
        .await;
        assert_eq!(done.status, 204);
        assert_eq!(audit_count("content.item.delete").await, 1);
    })
    .await;
}

#[tokio::test]
async fn cells_follow_the_column_order_and_exist_only_where_graded() {
    db_test(async |pool| {
        let w = world(pool).await;
        // Four columns in tree order: two in the module, one in its sub-module, one in the next module.
        let sub = module(&w, "Sub", Some(w.module)).await;
        let next = module(&w, "Next", None).await;
        let columns = [
            assignment_in(&w, w.module, "c0", "text", "10").await,
            assignment_in(&w, w.module, "c1", "text", "10").await,
            assignment_in(&w, sub, "c2", "no_submission", "10").await,
            assignment_in(&w, next, "c3", "text", "10").await,
        ];
        // Created in another order than they are listed in, to prove the order is the tree's.
        let students = [
            w.student.id,
            add_student(&w, "b@x.edu", "Bea", "B").await,
            add_student(&w, "c@x.edu", "Cal", "C").await,
            add_student(&w, "d@x.edu", "Dee", "D").await,
        ];
        set_name(&w, students[0], "Abe", "A").await;

        // Which columns each student has a grade row in: one has all four, one none.
        let graded: [&[usize]; 4] = [&[0, 1, 2, 3], &[1, 3], &[2], &[]];
        // Every score and feedback names its (student, column), so a cell moved to another place
        // is a different value.
        let score = |student: usize, column: usize| format!("{}.{}0", column + 1, student + 1);
        let note = |student: usize, column: usize| format!("s{student}c{column}");
        for (student, wanted) in graded.iter().enumerate() {
            // The student who has every cell is graded from the last column to the first.
            let order: Vec<usize> = if student == 0 {
                wanted.iter().rev().copied().collect()
            } else {
                wanted.to_vec()
            };
            for column in order {
                let sent = json!({"score": score(student, column), "feedback": note(student, column)});
                ok(&save(&w, &w.owner.token, columns[column], students[student], sent).await);
            }
        }

        let expected: Vec<Value> = graded
            .iter()
            .enumerate()
            .map(|(student, wanted)| {
                let cells: Vec<Value> = wanted
                    .iter()
                    .map(|&column| {
                        json!({
                            "assignment_id": columns[column],
                            "draft_score": score(student, column),
                            "draft_feedback": note(student, column),
                            "published_score": null,
                            "published_feedback": null,
                            "has_unpublished_changes": true,
                        })
                    })
                    .collect();
                let (name, surname) = [("Abe", "A"), ("Bea", "B"), ("Cal", "C"), ("Dee", "D")][student];
                json!({"user_id": students[student], "name": name, "surname": surname, "former": false, "cells": cells})
            })
            .collect();

        let book = gradebook(&w, &w.owner.token, "").await;
        let listed: Vec<_> = book["columns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["assignment_id"].clone())
            .collect();
        assert_eq!(
            listed,
            columns.map(|c| json!(c)),
            "the columns are in tree order"
        );
        assert_eq!(book["items"], json!(expected));

        // The same rows come back, in the same order, when the students are read two at a time.
        let mut paged = Vec::new();
        let mut cursor = None::<String>;
        loop {
            let query = match &cursor {
                None => "?limit=2".to_owned(),
                Some(cursor) => format!("?limit=2&cursor={cursor}"),
            };
            let page = gradebook(&w, &w.owner.token, &query).await;
            paged.extend(page["items"].as_array().unwrap().iter().cloned());
            match page["next_cursor"].as_str() {
                Some(next) => cursor = Some(next.to_owned()),
                None => break,
            }
        }
        assert_eq!(Value::Array(paged), json!(expected));
    })
    .await;
}
