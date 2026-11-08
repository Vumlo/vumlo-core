//! ADR-0074: what a plugin reads, and only from the courses it was granted. Every endpoint is
//! checked for the three refusals (no scope: 403; a course outside the grant: 403; an id that is not
//! there: 404), for what it must never show (drafts, the answer key, a draft grade, an address
//! outside the granted courses) and for its paging.

mod common;

use ::common::Role;
use access::{CourseScope, Scope};
use common::{Account, Env, db_test, id_of, spawn_app, test_settings};
use plugins::{Installed, parse_manifest};
use serde_json::{Value, json};
use server::app::AppState;
use sqlx::PgPool;
use uuid::Uuid;
use vumlo_config::Secret;

fn doc(text: &str) -> Value {
    json!({"v": 1, "attachments": [], "blocks": [
        {"id": "p1", "type": "paragraph", "content": [{"type": "text", "text": text}]}]})
}

fn quiz_config() -> Value {
    let opt = |id: &str| json!({"id": id, "content": doc(&format!("option {id}"))});
    json!({
        "v": 1, "shuffle_questions": false, "show_correct": true,
        "questions": [
            {"id": "q1", "type": "single_choice", "points": "2", "prompt": doc("one"),
             "options": [opt("a"), opt("b")], "correct": ["b"],
             "explanation": doc("because b"), "grading_notes": "SECRET-NOTE"},
            {"id": "q2", "type": "number", "points": "1", "prompt": doc("two"),
             "answer": "3.14", "tolerance": "0.01"},
        ]
    })
}

fn form_config() -> Value {
    json!({
        "v": 1,
        "fields": [
            {"id": "name", "type": "short", "label": "Team name", "required": true,
             "rules": {"max_length": 30}},
            {"id": "repo", "type": "link", "label": "Repository", "required": false},
        ]
    })
}

struct World {
    env: Env,
    state: AppState,
    teacher: Account,
    s1: Account,
    s2: Account,
    sb: Account,
    course: Uuid,
    module: Uuid,
    other_course: Uuid,
    other_module: Uuid,
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
    let teacher = env.user("teacher@x.edu", Role::Teacher).await;
    let s1 = env.user("s1@x.edu", Role::Student).await;
    let s2 = env.user("s2@x.edu", Role::Student).await;
    let sb = env.user("sb@x.edu", Role::Student).await;
    let (course, module) = env.course(&teacher, &[&s1, &s2]).await;
    let (other_course, other_module) = env.course(&teacher, &[&sb]).await;
    World {
        env,
        state,
        teacher,
        s1,
        s2,
        sb,
        course,
        module,
        other_course,
        other_module,
    }
}

impl World {
    /// A plugin that asked for every scope and was granted `scopes` and `courses`.
    async fn install(&self, id: &str, scopes: &[Scope], courses: CourseScope) -> Installed {
        let manifest = parse_manifest(
            json!({"id": id, "version": "1", "api": "1",
                   "scopes": Scope::ALL.map(Scope::as_str)})
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
            &courses,
        )
        .await
        .expect("the install")
    }

    /// A plugin with every scope and every course.
    async fn everything(&self) -> Installed {
        self.install("all", &Scope::ALL, CourseScope::All).await
    }

    async fn get(&self, token: &Installed, path: &str) -> common::Resp {
        common::Http::new(&self.env.base)
            .get(&format!("/plugin-api{path}"))
            .bearer(token.token.expose())
            .send()
            .await
    }

    async fn item(
        &self,
        module: Uuid,
        kind: &str,
        config: Value,
        points: &str,
        publish: bool,
    ) -> Uuid {
        let body = json!({"kind": "assignment", "title": format!("T-{kind}"),
            "assignment": {"type": kind, "config": config, "max_points": points}});
        let item = id_of(
            &self
                .env
                .ok(
                    "POST",
                    &self.teacher.token,
                    &format!("/modules/{module}/items"),
                    Some(body),
                    201,
                )
                .await,
        );
        if publish {
            self.env
                .ok(
                    "POST",
                    &self.teacher.token,
                    &format!("/items/{item}/publish"),
                    None,
                    200,
                )
                .await;
        }
        item
    }

    async fn text(&self, module: Uuid) -> Uuid {
        self.item(module, "text", json!({}), "10", true).await
    }

    async fn submit_text(&self, who: &Account, assignment: Uuid, text: &str) -> Uuid {
        let path = format!("/assignments/{assignment}/submission");
        self.env
            .ok(
                "PUT",
                &who.token,
                &format!("{path}/draft"),
                Some(json!({"content": common::essay(text)})),
                200,
            )
            .await;
        id_of(
            &self
                .env
                .ok("POST", &who.token, &format!("{path}/submit"), None, 201)
                .await,
        )
    }

    async fn submit_answers(&self, who: &Account, assignment: Uuid, answers: Value) -> Uuid {
        let path = format!("/assignments/{assignment}/submission");
        self.env
            .ok(
                "PUT",
                &who.token,
                &format!("{path}/draft"),
                Some(json!({"content": {"answers": answers}})),
                200,
            )
            .await;
        id_of(
            &self
                .env
                .ok("POST", &who.token, &format!("{path}/submit"), None, 201)
                .await,
        )
    }

    async fn grade(&self, assignment: Uuid, student: &Account, score: &str, publish: bool) {
        self.env
            .ok(
                "PUT",
                &self.teacher.token,
                &format!("/assignments/{assignment}/grades/{}", student.id),
                Some(json!({"score": score, "feedback": "PRIVATE-FEEDBACK"})),
                200,
            )
            .await;
        if publish {
            self.env
                .ok(
                    "POST",
                    &self.teacher.token,
                    &format!("/assignments/{assignment}/grades/publish"),
                    Some(json!({"students": "all"})),
                    200,
                )
                .await;
        }
    }
}

fn items(response: &common::Resp) -> Vec<Value> {
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    response.json()["items"].as_array().unwrap().clone()
}

fn ids(list: &[Value], key: &str) -> Vec<String> {
    list.iter()
        .map(|item| item[key].as_str().unwrap().to_owned())
        .collect()
}

// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn every_read_needs_its_scope_before_it_looks_for_anything() {
    db_test(async |pool| {
        let w = world(pool).await;
        let nothing = w.install("nothing", &[], CourseScope::All).await;
        let any = Uuid::now_v7();
        for path in [
            "/assignments".to_owned(),
            format!("/assignments/{any}"),
            format!("/submissions/{any}"),
            format!("/submissions/{any}/fields"),
            format!("/assignments/{any}/submissions"),
            format!("/courses/{any}/members"),
            "/grades".to_owned(),
            format!("/assignments/{any}/quiz"),
            format!("/assignments/{any}/attempts"),
            format!("/users/contact?ids={any}"),
        ] {
            // A real or an invented id: the answer is the same 403, so ids cannot be probed.
            let response = w.get(&nothing, &path).await;
            assert_eq!(response.status, 403, "{path}");
            response.problem(403);
        }
        // One scope opens only its own calls.
        let reader = w
            .install("reader", &[Scope::GradesRead], CourseScope::All)
            .await;
        assert_eq!(w.get(&reader, "/grades").await.status, 200);
        assert_eq!(w.get(&reader, "/assignments").await.status, 403);
        assert_eq!(
            w.get(&reader, &format!("/courses/{}/members", w.course))
                .await
                .status,
            403
        );
    })
    .await;
}

#[tokio::test]
async fn an_id_that_is_not_there_is_404_for_a_plugin_that_reaches_every_course() {
    db_test(async |pool| {
        let w = world(pool).await;
        let all = w.everything().await;
        let any = Uuid::now_v7();
        for path in [
            format!("/assignments/{any}"),
            format!("/submissions/{any}"),
            format!("/submissions/{any}/fields"),
            format!("/assignments/{any}/submissions"),
            format!("/courses/{any}/members"),
            format!("/grades?course_id={any}"),
            format!("/grades?assignment_id={any}"),
            format!("/assignments?course_id={any}"),
            format!("/assignments/{any}/quiz"),
            format!("/assignments/{any}/attempts"),
        ] {
            w.get(&all, &path).await.problem(404);
        }
        // A draft assignment is not one a student could see: it is not there.
        let draft = w.item(w.module, "text", json!({}), "10", false).await;
        w.get(&all, &format!("/assignments/{draft}"))
            .await
            .problem(404);
    })
    .await;
}

#[tokio::test]
async fn a_plugin_is_refused_the_courses_it_was_not_granted() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = w.text(w.module).await;
        let b = w.text(w.other_module).await;
        let sub_a = w.submit_text(&w.s1, a, "in a").await;
        let sub_b = w.submit_text(&w.sb, b, "in b").await;
        w.grade(a, &w.s1, "5", true).await;
        w.grade(b, &w.sb, "6", true).await;
        let only_a = w
            .install("only-a", &Scope::ALL, CourseScope::Only(vec![w.course]))
            .await;

        // What is in its course is read.
        for path in [
            format!("/assignments/{a}"),
            format!("/submissions/{sub_a}"),
            format!("/assignments/{a}/submissions"),
            format!("/courses/{}/members", w.course),
            format!("/grades?course_id={}", w.course),
            format!("/assignments?course_id={}", w.course),
        ] {
            assert_eq!(w.get(&only_a, &path).await.status, 200, "{path}");
        }
        // What is in the other course is 403, whatever the call.
        for path in [
            format!("/assignments/{b}"),
            format!("/submissions/{sub_b}"),
            format!("/assignments/{b}/submissions"),
            format!("/courses/{}/members", w.other_course),
            format!("/grades?course_id={}", w.other_course),
            format!("/grades?assignment_id={b}"),
            format!("/assignments?course_id={}", w.other_course),
        ] {
            w.get(&only_a, &path).await.problem(403);
        }
        // A list that names no course covers the granted ones and nothing else.
        let list = items(&w.get(&only_a, "/assignments").await);
        assert_eq!(ids(&list, "id"), [a.to_string()]);
        let grades = items(&w.get(&only_a, "/grades").await);
        assert_eq!(ids(&grades, "assignment_id"), [a.to_string()]);
        // A plugin granted no course at all reads nothing by course, and its lists are empty.
        let none = w
            .install("none", &Scope::ALL, CourseScope::Only(Vec::new()))
            .await;
        assert!(items(&w.get(&none, "/assignments").await).is_empty());
        w.get(&none, &format!("/assignments/{a}"))
            .await
            .problem(403);
    })
    .await;
}

#[tokio::test]
async fn assignments_are_read_as_a_student_sees_them_and_page_by_their_last_change() {
    db_test(async |pool| {
        let w = world(pool).await;
        let all = w.everything().await;
        let first = w.text(w.module).await;
        let form = w.item(w.module, "form", form_config(), "5", true).await;
        let _draft = w.item(w.module, "text", json!({}), "10", false).await;
        w.text(w.other_module).await;

        // The draft is not listed; the others are, with their kind, points and (for a form) fields.
        let list = items(
            &w.get(&all, &format!("/assignments?course_id={}", w.course))
                .await,
        );
        assert_eq!(list.len(), 2);
        let form_row = list.iter().find(|a| a["id"] == json!(form)).unwrap();
        assert_eq!(form_row["kind"], "form");
        assert_eq!(form_row["max_points"], "5.00");
        assert_eq!(form_row["fields"][0]["id"], "name");
        assert_eq!(form_row["fields"][0]["label"], "Team name");
        let text_row = list.iter().find(|a| a["id"] == json!(first)).unwrap();
        assert!(text_row["fields"].is_null());
        assert_eq!(text_row["course_id"], json!(w.course));
        // The same row by id.
        let one = w.get(&all, &format!("/assignments/{form}")).await.json();
        assert_eq!(one["fields"], form_row["fields"]);

        // Paging: one at a time, the cursor is opaque, and the pages add up to the whole list.
        let mut seen = Vec::new();
        let mut cursor = String::new();
        loop {
            let path = format!("/assignments?limit=1{cursor}");
            let page = w.get(&all, &path).await;
            let body = page.json();
            seen.extend(ids(&items(&page), "id"));
            match body["next_cursor"].as_str() {
                Some(next) => cursor = format!("&cursor={next}"),
                None => break,
            }
        }
        assert_eq!(seen.len(), 3, "{seen:?}");
        w.get(&all, "/assignments?cursor=junk").await.problem(422);
        w.get(&all, "/assignments?updated_since=yesterday")
            .await
            .problem(422);

        // `updated_since` is the catch-up: a change after the time shows up, older rows do not.
        let before = chrono::Utc::now();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        w.env
            .ok(
                "PATCH",
                &w.teacher.token,
                &format!("/items/{first}"),
                Some(json!({"due_at": "2099-01-01T10:00:00Z",
                    "on_deadline": "keep_draft", "accept_late": false})),
                200,
            )
            .await;
        let since = before.to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
        let changed = items(
            &w.get(&all, &format!("/assignments?updated_since={since}"))
                .await,
        );
        assert_eq!(ids(&changed, "id"), [first.to_string()]);
        assert!(
            changed[0]["due_at"]
                .as_str()
                .unwrap()
                .starts_with("2099-01-01")
        );
    })
    .await;
}

#[tokio::test]
async fn the_values_of_a_form_are_read_by_field_and_only_for_a_form() {
    db_test(async |pool| {
        let w = world(pool).await;
        let all = w.everything().await;
        let form = w.item(w.module, "form", form_config(), "5", true).await;
        let text = w.text(w.module).await;
        let filled = w
            .submit_answers(
                &w.s1,
                form,
                json!({"name": "Team Red", "repo": "https://x.test/r"}),
            )
            .await;
        let essay = w.submit_text(&w.s1, text, "words").await;

        let answers = w.get(&all, &format!("/submissions/{filled}/fields")).await;
        assert_eq!(
            answers.status,
            200,
            "{}",
            String::from_utf8_lossy(&answers.body)
        );
        let body = answers.json();
        assert_eq!(body["answers"]["name"], "Team Red");
        assert_eq!(body["student_id"], json!(w.s1.id));
        assert_eq!(body["assignment_id"], json!(form));
        // An essay has no fields.
        w.get(&all, &format!("/submissions/{essay}/fields"))
            .await
            .problem(404);
        // A draft over the submitted attempt is no attempt.
        w.env
            .ok(
                "PUT",
                &w.s1.token,
                &format!("/assignments/{form}/submission/draft"),
                Some(json!({"content": {"answers": {"name": "DRAFT-ONLY"}}})),
                200,
            )
            .await;
        let draft: Uuid =
            sqlx::query_scalar("SELECT id FROM assessment.submissions WHERE status = 'draft'")
                .fetch_one(&w.env.pool)
                .await
                .unwrap();
        w.get(&all, &format!("/submissions/{draft}/fields"))
            .await
            .problem(404);
        w.get(&all, &format!("/submissions/{draft}"))
            .await
            .problem(404);
    })
    .await;
}

#[tokio::test]
async fn the_roster_names_people_and_roles_and_holds_no_contact_data() {
    db_test(async |pool| {
        let w = world(pool).await;
        let all = w.everything().await;
        let path = format!("/courses/{}/members", w.course);
        let everyone = items(&w.get(&all, &path).await);
        let mut users = ids(&everyone, "user_id");
        users.sort();
        let mut expected = vec![
            w.teacher.id.to_string(),
            w.s1.id.to_string(),
            w.s2.id.to_string(),
        ];
        expected.sort();
        assert_eq!(users, expected);
        let raw = String::from_utf8_lossy(&w.get(&all, &path).await.body).to_string();
        assert!(!raw.contains("@x.edu") && !raw.contains("email"), "{raw}");
        assert!(everyone[0].get("name").is_some() && everyone[0].get("surname").is_some());

        let students = items(&w.get(&all, &format!("{path}?role=student")).await);
        assert_eq!(students.len(), 2);
        assert!(students.iter().all(|m| m["role"] == "student"));
        let teachers = items(&w.get(&all, &format!("{path}?role=teacher")).await);
        assert_eq!(ids(&teachers, "user_id"), [w.teacher.id.to_string()]);
        w.get(&all, &format!("{path}?role=boss")).await.problem(422);

        // A page at a time, by user id, to the end.
        let mut seen = Vec::new();
        let mut cursor = String::new();
        loop {
            let page = w.get(&all, &format!("{path}?limit=2{cursor}")).await;
            let next = page.json()["next_cursor"].as_str().map(str::to_owned);
            seen.extend(ids(&items(&page), "user_id"));
            match next {
                Some(next) => cursor = format!("&cursor={next}"),
                None => break,
            }
        }
        let mut sorted = seen.clone();
        sorted.sort();
        assert_eq!(seen, sorted, "sorted by user id");
        assert_eq!(seen.len(), 3);
    })
    .await;
}

#[tokio::test]
async fn submissions_are_listed_with_when_they_were_submitted_and_graded_and_never_a_draft() {
    db_test(async |pool| {
        let w = world(pool).await;
        let all = w.everything().await;
        let a = w.text(w.module).await;
        let one = w.submit_text(&w.s1, a, "first").await;
        let two = w.submit_text(&w.s2, a, "second").await;
        // A student's open draft is not in the list.
        w.env
            .ok(
                "PUT",
                &w.s1.token,
                &format!("/assignments/{a}/submission/draft"),
                Some(json!({"content": common::essay("only a draft")})),
                200,
            )
            .await;

        let path = format!("/assignments/{a}/submissions");
        let list = items(&w.get(&all, &path).await);
        assert_eq!(ids(&list, "id"), [one.to_string(), two.to_string()]);
        assert!(list.iter().all(|s| s["graded_at"].is_null()));
        assert_eq!(list[0]["attempt"], 1);
        assert_eq!(list[0]["is_late"], false);
        assert!(list[0].get("content").is_none());

        // A grade that is only a draft is not "graded"; a published one is, and says when.
        w.grade(a, &w.s1, "7", false).await;
        let list = items(&w.get(&all, &path).await);
        assert!(list.iter().all(|s| s["graded_at"].is_null()));
        w.env
            .ok(
                "POST",
                &w.teacher.token,
                &format!("/assignments/{a}/grades/publish"),
                Some(json!({"students": [w.s1.id]})),
                200,
            )
            .await;
        let list = items(&w.get(&all, &path).await);
        assert!(!list[0]["graded_at"].is_null());
        assert!(list[1]["graded_at"].is_null());

        // The single attempt carries its course and assignment now.
        let attempt = w.get(&all, &format!("/submissions/{one}")).await.json();
        assert_eq!(attempt["course_id"], json!(w.course));
        assert_eq!(attempt["assignment_id"], json!(a));

        // Paging and catching up by the time of submission.
        let first_page = w.get(&all, &format!("{path}?limit=1")).await;
        let cursor = first_page.json()["next_cursor"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(items(&first_page).len(), 1);
        let second = items(
            &w.get(&all, &format!("{path}?limit=1&cursor={cursor}"))
                .await,
        );
        assert_eq!(ids(&second, "id"), [two.to_string()]);
        let since = list[1]["submitted_at"]
            .as_str()
            .unwrap()
            .replace('+', "%2B");
        let late = items(&w.get(&all, &format!("{path}?updated_since={since}")).await);
        assert_eq!(ids(&late, "id"), [two.to_string()]);
    })
    .await;
}

#[tokio::test]
async fn grades_are_published_ones_only_and_say_who_and_when() {
    db_test(async |pool| {
        let w = world(pool).await;
        let all = w.everything().await;
        let a = w.text(w.module).await;
        w.submit_text(&w.s1, a, "x").await;
        w.submit_text(&w.s2, a, "y").await;
        w.grade(a, &w.s1, "8", true).await;
        // The second grade is a draft that was never published.
        w.env
            .ok(
                "PUT",
                &w.teacher.token,
                &format!("/assignments/{a}/grades/{}", w.s2.id),
                Some(json!({"score": "3", "feedback": "DRAFT-FEEDBACK"})),
                200,
            )
            .await;

        let response = w
            .get(&all, &format!("/grades?course_id={}", w.course))
            .await;
        let raw = String::from_utf8_lossy(&response.body).to_string();
        assert!(!raw.contains("FEEDBACK") && !raw.contains("draft"), "{raw}");
        let list = items(&response);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["student_id"], json!(w.s1.id));
        assert_eq!(list[0]["score"], "8.00");
        assert_eq!(list[0]["max_points"], "10.00");
        assert_eq!(list[0]["published_by"], json!(w.teacher.id));
        assert!(list[0]["published_at"].is_string());

        // By assignment, and with the time to catch up from.
        let by = items(&w.get(&all, &format!("/grades?assignment_id={a}")).await);
        assert_eq!(by.len(), 1);
        let after = list[0]["published_at"]
            .as_str()
            .unwrap()
            .replace('+', "%2B");
        // `>=`: the same grade again, and nothing before it.
        assert_eq!(
            items(&w.get(&all, &format!("/grades?updated_since={after}")).await).len(),
            1
        );
        w.grade(a, &w.s2, "4", true).await;
        let both = w.get(&all, "/grades?limit=1").await;
        assert_eq!(items(&both).len(), 1);
        let cursor = both.json()["next_cursor"].as_str().unwrap().to_owned();
        let rest = items(
            &w.get(&all, &format!("/grades?limit=1&cursor={cursor}"))
                .await,
        );
        assert_eq!(ids(&rest, "student_id"), [w.s2.id.to_string()]);
    })
    .await;
}

#[tokio::test]
async fn a_quiz_is_read_without_its_key_and_its_attempts_with_their_scores() {
    db_test(async |pool| {
        let w = world(pool).await;
        let all = w.everything().await;
        let quiz = w.item(w.module, "quiz", quiz_config(), "3", true).await;
        let text = w.text(w.module).await;
        let attempt = w
            .submit_answers(&w.s1, quiz, json!({"q1": ["b"], "q2": "3,14"}))
            .await;
        w.submit_answers(&w.s2, quiz, json!({"q1": ["a"], "q2": "1"}))
            .await;

        let response = w.get(&all, &format!("/assignments/{quiz}/quiz")).await;
        assert_eq!(
            response.status,
            200,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        let raw = String::from_utf8_lossy(&response.body).to_string();
        for secret in [
            "SECRET-NOTE",
            "\"correct\"",
            "\"explanation\"",
            "\"answer\"",
            "\"tolerance\"",
        ] {
            assert!(!raw.contains(secret), "{secret} is in {raw}");
        }
        let body = response.json();
        assert_eq!(body["config"]["questions"][0]["id"], "q1");
        assert_eq!(body["config"]["questions"][0]["options"][0]["id"], "a");
        assert_eq!(body["course_id"], json!(w.course));
        // Not a quiz: not here.
        w.get(&all, &format!("/assignments/{text}/quiz"))
            .await
            .problem(404);
        w.get(&all, &format!("/assignments/{text}/attempts"))
            .await
            .problem(404);

        let list = items(&w.get(&all, &format!("/assignments/{quiz}/attempts")).await);
        assert_eq!(list.len(), 2);
        let mine = list.iter().find(|a| a["id"] == json!(attempt)).unwrap();
        assert_eq!(mine["student_id"], json!(w.s1.id));
        assert_eq!(mine["scores"]["q1"], "2.00");
        assert_eq!(mine["scores"]["q2"], "1.00");
        let theirs = list.iter().find(|a| a["id"] != json!(attempt)).unwrap();
        assert_eq!(theirs["scores"]["q1"], "0.00");
        let started =
            chrono::DateTime::parse_from_rfc3339(mine["started_at"].as_str().unwrap()).unwrap();
        let submitted =
            chrono::DateTime::parse_from_rfc3339(mine["submitted_at"].as_str().unwrap()).unwrap();
        assert!(started <= submitted);
        let page = w
            .get(&all, &format!("/assignments/{quiz}/attempts?limit=1"))
            .await;
        assert!(page.json()["next_cursor"].is_string());
    })
    .await;
}

#[tokio::test]
async fn an_address_is_given_only_for_people_in_a_granted_course() {
    db_test(async |pool| {
        let w = world(pool).await;
        let all = w.everything().await;
        let only_a = w
            .install("only-a", &Scope::ALL, CourseScope::Only(vec![w.course]))
            .await;
        let stranger = w.env.user("stranger@x.edu", Role::Student).await;
        let wanted = |who: &[Uuid]| {
            who.iter()
                .map(Uuid::to_string)
                .collect::<Vec<_>>()
                .join(",")
        };
        let everyone = wanted(&[w.s1.id, w.sb.id, stranger.id, Uuid::now_v7()]);

        // Every course: those who are in one, not the stranger, not the id nobody has.
        let response = w.get(&all, &format!("/users/contact?ids={everyone}")).await;
        assert_eq!(response.header("cache-control"), "no-store");
        let got = items(&response);
        let mut emails: Vec<_> = got.iter().map(|c| c["email"].as_str().unwrap()).collect();
        emails.sort();
        assert_eq!(emails, ["s1@x.edu", "sb@x.edu"]);
        // Only course A: its members alone.
        let got = items(
            &w.get(&only_a, &format!("/users/contact?ids={everyone}"))
                .await,
        );
        assert_eq!(ids(&got, "user_id"), [w.s1.id.to_string()]);
        // Teachers have addresses too: that is who a plugin writes to.
        let got = items(
            &w.get(&only_a, &format!("/users/contact?ids={}", w.teacher.id))
                .await,
        );
        assert_eq!(got[0]["email"], "teacher@x.edu");

        // At most a hundred ids, and only ids.
        let many = wanted(&vec![Uuid::now_v7(); 101]);
        w.get(&all, &format!("/users/contact?ids={many}"))
            .await
            .problem(422);
        w.get(&all, "/users/contact?ids=not-an-id")
            .await
            .problem(422);
    })
    .await;
}

#[tokio::test]
async fn every_plugin_answer_is_no_store() {
    db_test(async |pool| {
        let w = world(pool).await;
        let all = w.everything().await;
        let a = w.text(w.module).await;
        let sub = w.submit_text(&w.s1, a, "x").await;
        for path in [
            "/assignments".to_owned(),
            format!("/assignments/{a}"),
            format!("/submissions/{sub}"),
            format!("/assignments/{a}/submissions"),
            format!("/courses/{}/members", w.course),
            "/grades".to_owned(),
        ] {
            let response = w.get(&all, &path).await;
            assert_eq!(response.status, 200, "{path}");
            assert_eq!(response.header("cache-control"), "no-store", "{path}");
        }
    })
    .await;
}

#[tokio::test]
async fn a_page_is_launched_only_for_an_assignment_in_a_course_the_plugin_reaches() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = w.text(w.module).await;
        let b = w.text(w.other_module).await;
        let manifest = parse_manifest(
            json!({"id": "paged", "version": "1", "api": "1", "ui": [
                {"slot": "assignment.tab", "title": "Page", "path": "/ui"}]})
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        let key = w.state.plugins.key.as_deref().unwrap();
        let mut conn = w.state.db.acquire().await.unwrap();
        plugins::install(
            &mut conn,
            key,
            "http://127.0.0.1:9",
            &manifest,
            &[],
            &CourseScope::Only(vec![w.course]),
        )
        .await
        .unwrap();
        let launch = async |assignment: Uuid| {
            let path = format!("/assignments/{assignment}/plugin-launch");
            w.env
                .call(
                    "POST",
                    &w.teacher.token,
                    &path,
                    Some(json!({"plugin_id": "paged"})),
                )
                .await
        };
        assert_eq!(launch(a).await.status, 200);
        launch(b).await.problem(404);
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// ADR-0081: what was deleted
// ---------------------------------------------------------------------------------------------

/// `deleted_since` an hour ago, as a query string.
fn since_an_hour_ago() -> String {
    since(chrono::Duration::hours(1))
}

fn since(ago: chrono::Duration) -> String {
    format!(
        "deleted_since={}",
        (chrono::Utc::now() - ago).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    )
}

impl World {
    async fn announce(&self, course: Uuid, title: &str) -> Uuid {
        id_of(
            &self
                .env
                .ok(
                    "POST",
                    &self.teacher.token,
                    &format!("/courses/{course}/announcements"),
                    Some(json!({"title": title, "body": "text"})),
                    201,
                )
                .await,
        )
    }
}

#[tokio::test]
async fn a_deleted_assignment_or_announcement_is_listed_with_when_it_went_and_nothing_else_is() {
    db_test(async |pool| {
        let w = world(pool).await;
        let all = w.everything().await;
        let published = w.text(w.module).await;
        let kept = w.text(w.module).await;
        let draft = w.item(w.module, "text", json!({}), "10", false).await;
        let lecture = id_of(
            &w.env
                .ok(
                    "POST",
                    &w.teacher.token,
                    &format!("/modules/{}/items", w.module),
                    Some(json!({"kind": "text", "title": "Lecture", "body": doc("hi")})),
                    201,
                )
                .await,
        );
        let announcement = w.announce(w.course, "gone").await;
        let remaining = w.announce(w.course, "stays").await;
        let q = since_an_hour_ago();
        for path in ["/assignments/deleted", "/announcements/deleted"] {
            assert!(
                items(&w.get(&all, &format!("{path}?{q}")).await).is_empty(),
                "nothing was deleted yet: {path}"
            );
        }

        for path in [
            format!("/items/{published}"),
            format!("/items/{draft}"),
            format!("/items/{lecture}"),
            format!("/announcements/{announcement}"),
        ] {
            w.env.ok("DELETE", &w.teacher.token, &path, None, 204).await;
        }
        // Only the published assignment: a draft was never shown, a text that is no assignment is no
        // assignment, and what stays is not there.
        let gone = items(&w.get(&all, &format!("/assignments/deleted?{q}")).await);
        assert_eq!(ids(&gone, "id"), [published.to_string()]);
        assert_eq!(gone[0]["course_id"], json!(w.course));
        let went_at: chrono::DateTime<chrono::Utc> =
            gone[0]["deleted_at"].as_str().unwrap().parse().unwrap();
        assert!((chrono::Utc::now() - went_at).num_seconds().abs() < 60);
        let gone = items(&w.get(&all, &format!("/announcements/deleted?{q}")).await);
        assert_eq!(ids(&gone, "id"), [announcement.to_string()]);
        let _ = (kept, remaining);

        // It is a list from a time: later than the deletion, it is empty.
        let later = format!(
            "deleted_since={}",
            (chrono::Utc::now() + chrono::Duration::minutes(5))
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        );
        assert!(items(&w.get(&all, &format!("/assignments/deleted?{later}")).await).is_empty());
        // And it says no more than that: no title, no body.
        assert_eq!(
            gone[0]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["course_id", "deleted_at", "id"]
        );
        // No store, like every answer to a plugin.
        let response = w.get(&all, &format!("/assignments/deleted?{q}")).await;
        assert_eq!(response.header("cache-control"), "no-store");
    })
    .await;
}

#[tokio::test]
async fn a_course_that_is_deleted_takes_its_assignments_and_announcements_with_it_into_the_list() {
    db_test(async |pool| {
        let w = world(pool).await;
        let all = w.everything().await;
        let only_a = w
            .install("only-a", &Scope::ALL, CourseScope::Only(vec![w.course]))
            .await;
        let in_b = w.text(w.other_module).await;
        let in_a = w.text(w.module).await;
        let note_b = w.announce(w.other_course, "b").await;
        // The course holds no submitted work, so it can be deleted.
        w.env
            .ok(
                "DELETE",
                &w.teacher.token,
                &format!("/courses/{}", w.other_course),
                None,
                204,
            )
            .await;
        let q = since_an_hour_ago();

        // The plugin that reaches every course hears of both, though the course is gone, and asking
        // for it by id is not a 404.
        let list = items(
            &w.get(
                &all,
                &format!("/assignments/deleted?{q}&course_id={}", w.other_course),
            )
            .await,
        );
        assert_eq!(ids(&list, "id"), [in_b.to_string()]);
        assert_eq!(list[0]["course_id"], json!(w.other_course));
        let list = items(&w.get(&all, &format!("/announcements/deleted?{q}")).await);
        assert_eq!(ids(&list, "id"), [note_b.to_string()]);
        // The plugin that was granted the other course does not, whether it names the course or not.
        w.get(
            &only_a,
            &format!("/assignments/deleted?{q}&course_id={}", w.other_course),
        )
        .await
        .problem(403);
        assert!(items(&w.get(&only_a, &format!("/assignments/deleted?{q}")).await).is_empty());
        assert!(items(&w.get(&only_a, &format!("/announcements/deleted?{q}")).await).is_empty());
        let _ = in_a;
    })
    .await;
}

#[tokio::test]
async fn an_assignment_with_work_cannot_be_deleted_so_it_is_never_in_the_list() {
    db_test(async |pool| {
        let w = world(pool).await;
        let all = w.everything().await;
        let worked = w.text(w.module).await;
        w.submit_text(&w.s1, worked, "handed in").await;
        let q = since_an_hour_ago();

        // Work that was handed in keeps the assignment: attempts and grades are never deleted, so
        // there is no list of deleted attempts or grades.
        w.env
            .call(
                "DELETE",
                &w.teacher.token,
                &format!("/items/{worked}"),
                None,
            )
            .await
            .problem(409);
        assert!(items(&w.get(&all, &format!("/assignments/deleted?{q}")).await).is_empty());
    })
    .await;
}

#[tokio::test]
async fn the_deleted_lists_need_their_scope_a_time_a_valid_cursor_and_a_time_the_core_still_keeps()
{
    db_test(async |pool| {
        let w = world(pool).await;
        let nothing = w.install("nothing", &[], CourseScope::All).await;
        let assignments_only = w
            .install("assign", &[Scope::AssignmentsRead], CourseScope::All)
            .await;
        let q = since_an_hour_ago();
        // The scope of the thing that was deleted, before anything else.
        for path in ["/assignments/deleted", "/announcements/deleted"] {
            w.get(&nothing, &format!("{path}?{q}")).await.problem(403);
        }
        assert_eq!(
            w.get(&assignments_only, &format!("/assignments/deleted?{q}"))
                .await
                .status,
            200
        );
        w.get(&assignments_only, &format!("/announcements/deleted?{q}"))
            .await
            .problem(403);

        let all = w.everything().await;
        // The time is not optional, and a cursor must be one the core gave.
        assert_eq!(w.get(&all, "/assignments/deleted").await.status, 422);
        assert_eq!(
            w.get(&all, "/assignments/deleted?deleted_since=yesterday")
                .await
                .status,
            422
        );
        assert_eq!(
            w.get(&all, &format!("/assignments/deleted?{q}&cursor=junk"))
                .await
                .status,
            422
        );
        // Further back than the 30 days the core keeps: `410`, so the plugin reads the list in full.
        let old = since(chrono::Duration::days(31));
        for path in ["/assignments/deleted", "/announcements/deleted"] {
            w.get(&all, &format!("{path}?{old}")).await.problem(410);
        }
        let within = since(chrono::Duration::days(29));
        assert_eq!(
            w.get(&all, &format!("/assignments/deleted?{within}"))
                .await
                .status,
            200
        );
    })
    .await;
}

#[tokio::test]
async fn deletions_page_by_the_time_they_happened_and_are_forgotten_after_their_retention() {
    db_test(async |pool| {
        let w = world(pool).await;
        let all = w.everything().await;
        let mut gone = Vec::new();
        for title in ["one", "two", "three"] {
            let id = w.announce(w.course, title).await;
            w.env
                .ok(
                    "DELETE",
                    &w.teacher.token,
                    &format!("/announcements/{id}"),
                    None,
                    204,
                )
                .await;
            gone.push(id.to_string());
        }
        let q = since_an_hour_ago();
        let first = w
            .get(&all, &format!("/announcements/deleted?{q}&limit=2"))
            .await;
        let next = first.json()["next_cursor"].as_str().unwrap().to_owned();
        let mut seen = ids(&items(&first), "id");
        assert_eq!(seen.len(), 2);
        let second = w
            .get(
                &all,
                &format!("/announcements/deleted?{q}&limit=2&cursor={next}"),
            )
            .await;
        assert!(second.json()["next_cursor"].is_null());
        seen.extend(ids(&items(&second), "id"));
        // All three, once each, in the order they went (ids are v7, so they sort the same way).
        assert_eq!(seen, gone);

        // The worker forgets what is older than `plugins.tombstone_days`; one row is made old.
        sqlx::query(
            "UPDATE teaching.tombstones SET deleted_at = now() - interval '40 days' WHERE id = $1::uuid",
        )
        .bind(&gone[0])
        .execute(&w.state.db)
        .await
        .unwrap();
        let deliverer = w
            .state
            .plugin_deliverer()
            .unwrap()
            .expect("the plugin key is set");
        assert_eq!(deliverer.prune(&w.state.db).await.unwrap(), 1);
        let rest = items(&w.get(&all, &format!("/announcements/deleted?{q}")).await);
        assert_eq!(ids(&rest, "id"), gone[1..]);
    })
    .await;
}
