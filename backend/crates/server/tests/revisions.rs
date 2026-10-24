//! ADR-0049: unpublished revisions, class days, per-item facts and the history of a grade.

mod common;

use ::common::Role;
use common::{Account, Env, db_test, essay, id_of};
use serde_json::{Value, json};
use uuid::Uuid;

struct World {
    env: Env,
    admin: Account,
    owner: Account,
    student: Account,
    other: Account,
    stranger: Account,
    course: Uuid,
    module: Uuid,
}

async fn world(pool: sqlx::PgPool) -> World {
    let env = Env::new(pool).await;
    let admin = env.user("admin@lms.test", Role::Admin).await;
    let owner = env.user("owner@lms.test", Role::Teacher).await;
    let student = env.user("student@lms.test", Role::Student).await;
    let other = env.user("other@lms.test", Role::Student).await;
    let stranger = env.user("stranger@lms.test", Role::Teacher).await;
    let (course, module) = env.course(&owner, &[&student, &other]).await;
    World {
        env,
        admin,
        owner,
        student,
        other,
        stranger,
        course,
        module,
    }
}

fn doc(text: &str) -> Value {
    json!({"v": 1, "attachments": [], "blocks": [
        {"id": "p1", "type": "paragraph", "content": [{"type": "text", "text": text}]}]})
}

impl World {
    async fn call(
        &self,
        method: &str,
        who: &Account,
        path: &str,
        body: Option<Value>,
    ) -> common::Resp {
        self.env.call(method, &who.token, path, body).await
    }

    async fn ok(
        &self,
        method: &str,
        who: &Account,
        path: &str,
        body: Option<Value>,
        status: u16,
    ) -> Value {
        self.env.ok(method, &who.token, path, body, status).await
    }

    /// A lecture in the module: published unless `draft`.
    async fn lecture(&self, title: &str, text: &str, draft: bool) -> Uuid {
        let item = id_of(
            &self
                .ok(
                    "POST",
                    &self.owner,
                    &format!("/modules/{}/items", self.module),
                    Some(json!({"kind": "text", "title": title, "body": doc(text)})),
                    201,
                )
                .await,
        );
        if !draft {
            self.ok(
                "POST",
                &self.owner,
                &format!("/items/{item}/publish"),
                None,
                200,
            )
            .await;
        }
        item
    }

    async fn assignment(&self, kind: &str, extra: Value) -> Uuid {
        let mut assignment = json!({"type": kind, "max_points": "20"});
        assignment
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let item = id_of(
            &self
                .ok(
                    "POST",
                    &self.owner,
                    &format!("/modules/{}/items", self.module),
                    Some(json!({"kind": "assignment", "title": "Task", "assignment": assignment})),
                    201,
                )
                .await,
        );
        self.ok(
            "POST",
            &self.owner,
            &format!("/items/{item}/publish"),
            None,
            200,
        )
        .await;
        item
    }

    async fn meta(&self, who: &Account) -> Value {
        self.ok(
            "GET",
            who,
            &format!("/courses/{}/item-meta", self.course),
            None,
            200,
        )
        .await["items"]
            .clone()
    }

    async fn tree_titles(&self, who: &Account) -> Vec<String> {
        let tree = self
            .ok(
                "GET",
                who,
                &format!("/courses/{}/content", self.course),
                None,
                200,
            )
            .await;
        tree["modules"][0]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["title"].as_str().unwrap().to_owned())
            .collect()
    }

    async fn submit(&self, who: &Account, assignment: Uuid) {
        let base = format!("/assignments/{assignment}/submission");
        self.ok(
            "PUT",
            who,
            &format!("{base}/draft"),
            Some(json!({"content": essay("answer")})),
            200,
        )
        .await;
        self.ok("POST", who, &format!("{base}/submit"), None, 201)
            .await;
    }

    async fn grade(&self, assignment: Uuid, student: &Account, score: &str) {
        self.ok(
            "PUT",
            &self.owner,
            &format!("/assignments/{assignment}/grades/{}", student.id),
            Some(json!({"score": score, "feedback": "ok"})),
            200,
        )
        .await;
    }

    async fn publish_grades(&self, assignment: Uuid) {
        self.ok(
            "POST",
            &self.owner,
            &format!("/assignments/{assignment}/grades/publish"),
            Some(json!({"students": "all"})),
            200,
        )
        .await;
    }
}

// ---------------------------------------------------------------------------------------------
// Revisions
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn students_keep_reading_the_published_title_and_body_until_the_revision_is_published() {
    db_test(async |pool| {
        let w = world(pool).await;
        let item = w.lecture("Lecture 1", "the first text", false).await;
        let path = format!("/items/{item}");

        w.ok(
            "PATCH",
            &w.owner,
            &path,
            Some(json!({"title": "Lecture 1, revised", "body": doc("the second text")})),
            200,
        )
        .await;

        // Staff read the working copy, students the published one, everywhere.
        let staff = w.ok("GET", &w.owner, &path, None, 200).await;
        assert_eq!(staff["title"], "Lecture 1, revised");
        assert_eq!(
            staff["body"]["blocks"][0]["content"][0]["text"],
            "the second text"
        );
        let student = w.ok("GET", &w.student, &path, None, 200).await;
        assert_eq!(student["title"], "Lecture 1");
        assert_eq!(
            student["body"]["blocks"][0]["content"][0]["text"],
            "the first text"
        );
        assert_eq!(w.tree_titles(&w.owner).await, ["Lecture 1, revised"]);
        assert_eq!(w.tree_titles(&w.student).await, ["Lecture 1"]);

        let revision = w
            .ok("GET", &w.owner, &format!("{path}/revision"), None, 200)
            .await;
        assert_eq!(revision["has_unpublished_changes"], true);
        assert!(revision["last_published_at"].is_string());
        assert!(revision["size_bytes"].as_i64().unwrap() > 0);
        assert_eq!(w.meta(&w.owner).await[item.to_string()]["edited"], true);
        assert!(
            w.meta(&w.student).await[item.to_string()]
                .get("edited")
                .is_none()
        );

        let published = w
            .ok(
                "POST",
                &w.owner,
                &format!("{path}/revision/publish"),
                None,
                200,
            )
            .await;
        assert_eq!(published["has_unpublished_changes"], false);

        let student = w.ok("GET", &w.student, &path, None, 200).await;
        assert_eq!(student["title"], "Lecture 1, revised");
        assert_eq!(
            student["body"]["blocks"][0]["content"][0]["text"],
            "the second text"
        );
        assert_eq!(w.tree_titles(&w.student).await, ["Lecture 1, revised"]);
        let again = w
            .ok("GET", &w.owner, &format!("{path}/revision"), None, 200)
            .await;
        assert_eq!(again["has_unpublished_changes"], false);
        assert!(
            w.meta(&w.owner).await[item.to_string()]
                .get("edited")
                .is_none()
        );
    })
    .await;
}

#[tokio::test]
async fn publishing_an_item_for_the_first_time_gives_students_its_latest_text_and_a_draft_has_no_revision()
 {
    db_test(async |pool| {
        let w = world(pool).await;
        let item = w.lecture("Draft title", "v1", true).await;
        let path = format!("/items/{item}");
        w.ok(
            "PATCH",
            &w.owner,
            &path,
            Some(json!({"title": "Final title", "body": doc("v2")})),
            200,
        )
        .await;

        // A draft: no published copy to compare with, and nothing to publish a revision of.
        let revision = w
            .ok("GET", &w.owner, &format!("{path}/revision"), None, 200)
            .await;
        assert_eq!(revision["has_unpublished_changes"], false);
        assert_eq!(revision["last_published_at"], Value::Null);
        w.call("POST", &w.owner, &format!("{path}/revision/publish"), None)
            .await
            .problem(409);
        w.ok("GET", &w.student, &path, None, 404).await;

        w.ok("POST", &w.owner, &format!("{path}/publish"), None, 200)
            .await;

        let student = w.ok("GET", &w.student, &path, None, 200).await;
        assert_eq!(student["title"], "Final title");
        assert_eq!(student["body"]["blocks"][0]["content"][0]["text"], "v2");
    })
    .await;
}

#[tokio::test]
async fn publishing_a_module_does_not_publish_the_pending_edits_of_items_already_published() {
    db_test(async |pool| {
        let w = world(pool).await;
        let live = w.lecture("Live", "live text", false).await;
        let draft = w.lecture("Draft", "draft text", true).await;
        w.ok(
            "PATCH",
            &w.owner,
            &format!("/items/{live}"),
            Some(json!({"title": "Live, edited"})),
            200,
        )
        .await;
        w.ok(
            "PATCH",
            &w.owner,
            &format!("/items/{draft}"),
            Some(json!({"title": "Draft, edited"})),
            200,
        )
        .await;

        w.ok(
            "POST",
            &w.owner,
            &format!("/modules/{}/publish", w.module),
            Some(json!({"cascade": true})),
            200,
        )
        .await;

        let titles = w.tree_titles(&w.student).await;
        assert!(titles.contains(&"Live".to_owned()), "{titles:?}");
        assert!(titles.contains(&"Draft, edited".to_owned()), "{titles:?}");
        assert!(!titles.contains(&"Live, edited".to_owned()), "{titles:?}");
    })
    .await;
}

#[tokio::test]
async fn the_title_a_student_sees_on_their_grades_is_the_published_one() {
    db_test(async |pool| {
        let w = world(pool).await;
        let task = w.assignment("text", json!({})).await;
        w.submit(&w.student, task).await;
        w.grade(task, &w.student, "15").await;
        w.publish_grades(task).await;
        w.ok(
            "PATCH",
            &w.owner,
            &format!("/items/{task}"),
            Some(json!({"title": "Task, retitled"})),
            200,
        )
        .await;
        let grades = format!("/courses/{}/my-grades", w.course);

        assert_eq!(
            w.ok("GET", &w.student, &grades, None, 200).await["items"][0]["title"],
            "Task"
        );

        w.ok(
            "POST",
            &w.owner,
            &format!("/items/{task}/revision/publish"),
            None,
            200,
        )
        .await;
        assert_eq!(
            w.ok("GET", &w.student, &grades, None, 200).await["items"][0]["title"],
            "Task, retitled"
        );
    })
    .await;
}

#[tokio::test]
async fn points_and_deadlines_are_not_revisioned_and_apply_at_once() {
    db_test(async |pool| {
        let w = world(pool).await;
        let task = w.assignment("text", json!({})).await;

        w.ok(
            "PATCH",
            &w.owner,
            &format!("/items/{task}"),
            Some(json!({"max_points": "30"})),
            200,
        )
        .await;

        let seen = w
            .ok("GET", &w.student, &format!("/items/{task}"), None, 200)
            .await;
        assert_eq!(seen["assignment"]["max_points"], "30.00");
        let revision = w
            .ok(
                "GET",
                &w.owner,
                &format!("/items/{task}/revision"),
                None,
                200,
            )
            .await;
        assert_eq!(revision["has_unpublished_changes"], false);
    })
    .await;
}

#[tokio::test]
async fn only_staff_read_and_publish_revisions_and_an_archived_course_takes_none() {
    db_test(async |pool| {
        let w = world(pool).await;
        let item = w.lecture("L", "text", false).await;
        let path = format!("/items/{item}/revision");

        assert_eq!(w.call("GET", &w.student, &path, None).await.status, 403);
        assert_eq!(
            w.call("POST", &w.student, &format!("{path}/publish"), None)
                .await
                .status,
            403
        );
        assert_eq!(w.call("GET", &w.stranger, &path, None).await.status, 404);
        assert_eq!(
            w.call("POST", &w.stranger, &format!("{path}/publish"), None)
                .await
                .status,
            404
        );
        w.ok(
            "GET",
            &w.owner,
            &format!("/items/{}/revision", Uuid::now_v7()),
            None,
            404,
        )
        .await;

        // An admin's publish is audited.
        w.ok("POST", &w.admin, &format!("{path}/publish"), None, 200)
            .await;
        let actions = sqlx::query_scalar!("SELECT action FROM events.audit_log")
            .fetch_all(&w.env.pool)
            .await
            .unwrap();
        assert!(
            actions.contains(&"content.item.revision_publish".to_owned()),
            "{actions:?}"
        );

        w.ok(
            "POST",
            &w.owner,
            &format!("/courses/{}/archive", w.course),
            None,
            200,
        )
        .await;
        let refused = w
            .call("POST", &w.owner, &format!("{path}/publish"), None)
            .await;
        assert_eq!(
            refused.problem(409).detail.as_deref(),
            Some("the course is archived")
        );
        assert_eq!(w.call("GET", &w.owner, &path, None).await.status, 200);
    })
    .await;
}

#[tokio::test]
async fn existing_items_keep_what_their_students_read_when_the_slots_are_added() {
    db_test(async |pool| {
        // A published item has both slots, equal to the working copy at the moment of publishing.
        let w = world(pool).await;
        let item = w.lecture("Same", "same text", false).await;
        let row = sqlx::query!(
            r#"SELECT title, published_title, last_published_at IS NOT NULL AS "stamped!"
               FROM content.items WHERE id = $1"#,
            item
        )
        .fetch_one(&w.env.pool)
        .await
        .unwrap();
        assert_eq!(row.published_title.as_deref(), Some(row.title.as_str()));
        assert!(row.stamped);
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// Class day
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn an_in_class_assignment_has_a_class_day_that_is_set_changed_and_cleared() {
    db_test(async |pool| {
        let w = world(pool).await;
        let oral = w
            .assignment(
                "no_submission",
                json!({"max_points": "12", "class_on": "2026-10-29"}),
            )
            .await;
        let path = format!("/items/{oral}");

        let created = w.ok("GET", &w.owner, &path, None, 200).await;
        assert_eq!(created["assignment"]["class_on"], "2026-10-29");
        // Both the staff and the student read it in the item facts.
        for who in [&w.owner, &w.student] {
            assert_eq!(
                w.meta(who).await[oral.to_string()]["class_on"],
                "2026-10-29"
            );
        }

        let moved = w
            .ok(
                "PATCH",
                &w.owner,
                &path,
                Some(json!({"class_on": "2026-11-05"})),
                200,
            )
            .await;
        assert_eq!(moved["assignment"]["class_on"], "2026-11-05");
        let kept = w
            .ok(
                "PATCH",
                &w.owner,
                &path,
                Some(json!({"title": "Oral"})),
                200,
            )
            .await;
        assert_eq!(
            kept["assignment"]["class_on"], "2026-11-05",
            "an absent field keeps it"
        );
        let cleared = w
            .ok(
                "PATCH",
                &w.owner,
                &path,
                Some(json!({"class_on": null})),
                200,
            )
            .await;
        assert_eq!(cleared["assignment"]["class_on"], Value::Null);
    })
    .await;
}

#[tokio::test]
async fn work_that_takes_submissions_has_no_class_day() {
    db_test(async |pool| {
        let w = world(pool).await;
        let create = |class_on: Value| {
            json!({"kind": "assignment", "title": "T",
                   "assignment": {"type": "text", "max_points": "10", "class_on": class_on}})
        };

        let refused = w
            .call(
                "POST",
                &w.owner,
                &format!("/modules/{}/items", w.module),
                Some(create(json!("2026-10-29"))),
            )
            .await;
        let problem = refused.problem(422);
        assert_eq!(problem.errors[0].field, "assignment.class_on");

        let task = w.assignment("text", json!({})).await;
        let refused = w
            .call(
                "PATCH",
                &w.owner,
                &format!("/items/{task}"),
                Some(json!({"class_on": "2026-10-29"})),
            )
            .await;
        assert_eq!(refused.problem(422).errors[0].field, "class_on");
        // Clearing what is not there is harmless.
        w.ok(
            "PATCH",
            &w.owner,
            &format!("/items/{task}"),
            Some(json!({"class_on": null})),
            200,
        )
        .await;
        // A text item has no assignment fields at all.
        let lecture = w.lecture("L", "x", false).await;
        let refused = w
            .call(
                "PATCH",
                &w.owner,
                &format!("/items/{lecture}"),
                Some(json!({"class_on": "2026-10-29"})),
            )
            .await;
        assert_eq!(refused.problem(422).errors[0].field, "class_on");
        let bad = w
            .call(
                "PATCH",
                &w.owner,
                &format!("/items/{task}"),
                Some(json!({"class_on": "not a date"})),
            )
            .await;
        bad.problem(422);
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// Item facts
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_lecture_takes_a_minute_per_two_hundred_words_and_at_least_one() {
    db_test(async |pool| {
        let w = world(pool).await;
        let short = w.lecture("Short", "just a few words", false).await;
        let long = w.lecture("Long", &"word ".repeat(450), false).await;
        let empty = w.lecture("Empty", "", false).await;

        let meta = w.meta(&w.student).await;

        assert_eq!(meta[short.to_string()]["read_minutes"], 1);
        assert_eq!(meta[long.to_string()]["read_minutes"], 3);
        assert_eq!(meta[empty.to_string()]["read_minutes"], 1);

        // A student is timed on what they read: the published body, not the pending rewrite.
        w.ok(
            "PATCH",
            &w.owner,
            &format!("/items/{short}"),
            Some(json!({"body": doc(&"word ".repeat(450))})),
            200,
        )
        .await;
        assert_eq!(
            w.meta(&w.student).await[short.to_string()]["read_minutes"],
            1
        );
        assert_eq!(w.meta(&w.owner).await[short.to_string()]["read_minutes"], 3);
    })
    .await;
}

#[tokio::test]
async fn a_student_sees_their_own_standing_with_published_values_only() {
    db_test(async |pool| {
        let w = world(pool).await;
        let task = w.assignment("text", json!({"max_attempts": 3})).await;
        let key = task.to_string();
        let standing = || async { w.meta(&w.student).await[task.to_string()].clone() };

        assert_eq!(standing().await["status"], "not_started");
        assert_eq!(standing().await["attempts_used"], 0);

        w.ok(
            "PUT",
            &w.student,
            &format!("/assignments/{task}/submission/draft"),
            Some(json!({"content": essay("work in progress")})),
            200,
        )
        .await;
        let draft = standing().await;
        assert_eq!(draft["status"], "draft");
        assert!(draft["draft_saved_at"].is_string());

        w.submit(&w.student, task).await;
        assert_eq!(standing().await["status"], "submitted");
        assert_eq!(standing().await["attempts_used"], 1);

        // A draft grade is not the student's business.
        w.grade(task, &w.student, "17").await;
        let before = standing().await;
        assert_eq!(before["status"], "submitted");
        assert!(
            before.get("score").is_none() && before.get("published_at").is_none(),
            "{before}"
        );
        let text = w.meta(&w.student).await.to_string();
        assert!(!text.contains("17.00"), "{text}");
        for staff_only in [
            "to_grade",
            "draft_grades",
            "graded_total",
            "submitted",
            "edited",
        ] {
            assert!(before.get(staff_only).is_none(), "{staff_only}");
        }

        w.publish_grades(task).await;
        let after = standing().await;
        assert_eq!(after["status"], "graded");
        assert_eq!(after["score"], "17.00");
        assert!(after["published_at"].is_string());
        // The other student has none of it.
        assert_eq!(w.meta(&w.other).await[&key]["status"], "not_started");
    })
    .await;
}

#[tokio::test]
async fn in_class_work_and_a_closed_window_have_their_own_statuses() {
    db_test(async |pool| {
        let w = world(pool).await;
        let oral = w.assignment("no_submission", json!({"max_points": "12"})).await;
        let late_closed = w
            .assignment(
                "text",
                json!({"due_at": "2020-01-01T00:00:00Z", "on_deadline": "keep_draft", "accept_late": false}),
            )
            .await;
        let late_open = w
            .assignment(
                "text",
                json!({"due_at": "2020-01-01T00:00:00Z", "on_deadline": "keep_draft", "accept_late": true}),
            )
            .await;
        let future = w
            .assignment(
                "text",
                json!({"due_at": "2099-01-01T00:00:00Z", "on_deadline": "keep_draft", "accept_late": false}),
            )
            .await;

        let meta = w.meta(&w.student).await;

        assert_eq!(meta[oral.to_string()]["status"], "in_class");
        assert_eq!(meta[late_closed.to_string()]["status"], "missed");
        assert_eq!(meta[late_open.to_string()]["status"], "not_started", "late work is still taken");
        assert_eq!(meta[future.to_string()]["status"], "not_started");
    })
    .await;
}

#[tokio::test]
async fn staff_see_what_is_left_to_grade_and_what_is_not_yet_published() {
    db_test(async |pool| {
        let w = world(pool).await;
        let task = w.assignment("text", json!({})).await;
        let counts = || async { w.meta(&w.owner).await[task.to_string()].clone() };

        let start = counts().await;
        assert_eq!(
            (
                start["submitted"].as_u64(),
                start["to_grade"].as_u64(),
                start["draft_grades"].as_u64(),
                start["graded_total"].as_u64()
            ),
            (Some(0), Some(0), Some(0), Some(2))
        );

        w.submit(&w.student, task).await;
        w.submit(&w.other, task).await;
        let submitted = counts().await;
        assert_eq!(
            (
                submitted["submitted"].as_u64(),
                submitted["to_grade"].as_u64()
            ),
            (Some(2), Some(2))
        );

        w.grade(task, &w.student, "10").await;
        let graded = counts().await;
        assert_eq!(
            (graded["to_grade"].as_u64(), graded["draft_grades"].as_u64()),
            (Some(1), Some(1)),
            "one score is waiting, one draft is unpublished"
        );

        w.publish_grades(task).await;
        let published = counts().await;
        assert_eq!(
            (
                published["to_grade"].as_u64(),
                published["draft_grades"].as_u64()
            ),
            (Some(1), Some(0))
        );
        // The staff see no student-only fields.
        assert!(published.get("status").is_none() && published.get("attempts_used").is_none());
    })
    .await;
}

#[tokio::test]
async fn the_facts_only_cover_what_the_caller_may_see() {
    db_test(async |pool| {
        let w = world(pool).await;
        let visible = w.lecture("Visible", "text", false).await;
        let hidden = w.lecture("Hidden", "text", true).await;
        let path = format!("/courses/{}/item-meta", w.course);

        let student = w.meta(&w.student).await;
        assert!(student.get(visible.to_string()).is_some());
        assert!(student.get(hidden.to_string()).is_none());
        assert!(w.meta(&w.owner).await.get(hidden.to_string()).is_some());
        w.ok("GET", &w.stranger, &path, None, 404).await;
        w.ok(
            "GET",
            &w.owner,
            &format!("/courses/{}/item-meta", Uuid::now_v7()),
            None,
            404,
        )
        .await;
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// Grade history
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_history_of_a_grade_lists_every_save_and_publish_newest_first_with_the_person() {
    db_test(async |pool| {
        let w = world(pool).await;
        let task = w.assignment("text", json!({})).await;
        w.grade(task, &w.student, "10").await;
        w.grade(task, &w.student, "12.5").await;
        w.publish_grades(task).await;
        let path = format!("/assignments/{task}/grades/{}/history", w.student.id);

        let page = w.ok("GET", &w.owner, &path, None, 200).await;

        let entries = page["items"].as_array().unwrap();
        let actions: Vec<_> = entries
            .iter()
            .map(|e| e["action"].as_str().unwrap())
            .collect();
        assert_eq!(actions, ["publish", "save_draft", "save_draft"]);
        let scores: Vec<_> = entries
            .iter()
            .map(|e| e["score"].as_str().unwrap())
            .collect();
        assert_eq!(scores, ["12.50", "12.50", "10.00"]);
        assert_eq!(entries[0]["feedback"], "ok");
        assert_eq!(entries[0]["actor"]["user_id"], w.owner.id.to_string());
        assert!(entries[0]["actor"]["name"].is_string());
        assert!(entries[0]["at"].is_string());
        assert_eq!(page["next_cursor"], Value::Null);

        // Paged by id, newest first.
        let first = w
            .ok("GET", &w.owner, &format!("{path}?limit=2"), None, 200)
            .await;
        assert_eq!(first["items"].as_array().unwrap().len(), 2);
        let cursor = first["next_cursor"].as_str().unwrap();
        let rest = w
            .ok(
                "GET",
                &w.owner,
                &format!("{path}?limit=2&cursor={cursor}"),
                None,
                200,
            )
            .await;
        assert_eq!(rest["items"].as_array().unwrap().len(), 1);
        assert_eq!(rest["items"][0]["score"], "10.00");

        // Another student's history is their own.
        let other = format!("/assignments/{task}/grades/{}/history", w.other.id);
        assert!(
            w.ok("GET", &w.owner, &other, None, 200).await["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    })
    .await;
}

#[tokio::test]
async fn a_cleared_grade_is_recorded_as_a_row_without_values_and_students_cannot_read_history() {
    db_test(async |pool| {
        let w = world(pool).await;
        let task = w.assignment("text", json!({})).await;
        w.grade(task, &w.student, "10").await;
        w.ok(
            "PUT",
            &w.owner,
            &format!("/assignments/{task}/grades/{}", w.student.id),
            Some(json!({"score": null, "feedback": null})),
            200,
        )
        .await;
        let path = format!("/assignments/{task}/grades/{}/history", w.student.id);

        let page = w.ok("GET", &w.owner, &path, None, 200).await;

        assert_eq!(page["items"][0]["score"], Value::Null);
        assert_eq!(page["items"][0]["feedback"], Value::Null);
        assert_eq!(w.call("GET", &w.student, &path, None).await.status, 403);
        assert_eq!(w.call("GET", &w.stranger, &path, None).await.status, 404);
        w.ok(
            "GET",
            &w.owner,
            &format!(
                "/assignments/{}/grades/{}/history",
                Uuid::now_v7(),
                w.student.id
            ),
            None,
            404,
        )
        .await;
        w.call("GET", &w.owner, &format!("{path}?cursor=nope"), None)
            .await
            .problem(422);
    })
    .await;
}

#[tokio::test]
async fn every_endpoint_of_this_task_is_in_the_openapi_document() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let doc = env.http.get("/openapi.json").send().await.json();
        for (path, method) in [
            ("/api/v1/items/{id}/revision", "get"),
            ("/api/v1/items/{id}/revision/publish", "post"),
            ("/api/v1/courses/{id}/item-meta", "get"),
            (
                "/api/v1/assignments/{id}/grades/{student_id}/history",
                "get",
            ),
        ] {
            assert!(doc["paths"][path][method].is_object(), "{method} {path}");
        }
        assert!(doc["components"]["schemas"]["ItemMeta"].is_object());
    })
    .await;
}
