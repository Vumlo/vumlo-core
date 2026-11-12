//! ADR-0091: the scheduler. An assignment is announced to the plugins when students can first see
//! it, which `publish_at` or the state of a module or course can put after its publish; a draft is
//! submitted at the deadline when the assignment says so. Each test runs a tick at the moment it
//! chooses, so none of them waits.

mod common;

use ::common::Role;
use chrono::{DateTime, Duration, Utc};
use common::{Account, Env, db_test, id_of, spawn_app, test_settings};
use learning::schedule::{Tick, tick};
use serde_json::{Value, json};
use server::app::AppState;
use sqlx::PgPool;
use uuid::Uuid;

struct World {
    env: Env,
    state: AppState,
    teacher: Account,
    student: Account,
    course: Uuid,
    module: Uuid,
}

async fn world(pool: PgPool) -> World {
    let (base, state) = spawn_app(pool.clone(), test_settings()).await;
    let env = Env {
        http: common::Http::new(&base),
        pool,
        base,
    };
    let teacher = env.user("teacher@x.edu", Role::Teacher).await;
    let student = env.user("student@x.edu", Role::Student).await;
    let (course, module) = env.course(&teacher, &[&student]).await;
    World {
        env,
        state,
        teacher,
        student,
        course,
        module,
    }
}

fn essay(text: &str) -> Value {
    common::essay(text)
}

fn quiz_config() -> Value {
    json!({"v": 1, "shuffle_questions": false, "show_correct": false, "questions": [
        {"id": "q1", "type": "number", "points": "1", "prompt": essay("one"),
         "answer": "3", "tolerance": "0"}]})
}

fn auto_submit_at(due: DateTime<Utc>) -> Value {
    json!({"due_at": due.to_rfc3339(), "on_deadline": "auto_submit", "accept_late": false})
}

impl World {
    async fn go(&self, who: &Account, method: &str, path: &str, body: Option<Value>) -> Value {
        self.env.ok(method, &who.token, path, body, 200).await
    }

    /// A draft assignment in `module` with `extra` merged into the assignment object.
    async fn draft_in(&self, module: Uuid, kind: &str, config: Value, extra: Value) -> Uuid {
        let mut assignment = json!({"type": kind, "max_points": "10", "config": config});
        for (key, value) in extra.as_object().unwrap() {
            assignment[key] = value.clone();
        }
        id_of(
            &self
                .env
                .ok(
                    "POST",
                    &self.teacher.token,
                    &format!("/modules/{module}/items"),
                    Some(json!({"kind": "assignment", "title": "A", "assignment": assignment})),
                    201,
                )
                .await,
        )
    }

    async fn publish(&self, item: Uuid) {
        self.go(
            &self.teacher,
            "POST",
            &format!("/items/{item}/publish"),
            None,
        )
        .await;
    }

    async fn published(&self, kind: &str, config: Value, extra: Value) -> Uuid {
        let item = self.draft_in(self.module, kind, config, extra).await;
        self.publish(item).await;
        item
    }

    async fn save(&self, assignment: Uuid, content: Value) {
        self.go(
            &self.student,
            "PUT",
            &format!("/assignments/{assignment}/submission/draft"),
            Some(json!({"content": content})),
        )
        .await;
    }

    async fn mine(&self, assignment: Uuid) -> Value {
        self.go(
            &self.student,
            "GET",
            &format!("/assignments/{assignment}/submission"),
            None,
        )
        .await
    }

    async fn tick(&self, at: DateTime<Utc>) -> Tick {
        tick(&self.state.db, &self.state.learning.editor, at)
            .await
            .expect("a tick")
    }

    /// How many events of `kind` the core has written, delivered or not.
    async fn events(&self, kind: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM events.outbox WHERE type = $1")
            .bind(kind)
            .fetch_one(&self.env.pool)
            .await
            .unwrap()
    }
}

// --- announcing an assignment when it becomes visible -------------------------------------------

#[tokio::test]
async fn an_assignment_scheduled_for_later_is_announced_when_its_time_comes() {
    db_test(async |pool| {
        let w = world(pool).await;
        let now = Utc::now();
        let item = w.draft_in(w.module, "text", json!({}), json!({})).await;
        w.go(
            &w.teacher,
            "PATCH",
            &format!("/items/{item}"),
            Some(json!({"publish_at": (now + Duration::hours(1)).to_rfc3339()})),
        )
        .await;
        w.publish(item).await;

        // Published, but students cannot see it yet: that is not news.
        assert_eq!(w.events("assignment.created").await, 0);
        assert_eq!(w.tick(now).await, Tick::default());
        assert_eq!(w.events("assignment.created").await, 0);

        // Its time comes: it is announced once, however many ticks follow.
        let later = now + Duration::hours(2);
        assert_eq!(w.tick(later).await.announced, 1);
        assert_eq!(w.tick(later + Duration::hours(1)).await.announced, 0);
        assert_eq!(w.events("assignment.created").await, 1);
    })
    .await;
}

#[tokio::test]
async fn clearing_publish_at_makes_the_next_tick_announce_the_assignment() {
    db_test(async |pool| {
        let w = world(pool).await;
        let item = w.draft_in(w.module, "text", json!({}), json!({})).await;
        w.go(
            &w.teacher,
            "PATCH",
            &format!("/items/{item}"),
            Some(json!({"publish_at": (Utc::now() + Duration::days(1)).to_rfc3339()})),
        )
        .await;
        w.publish(item).await;
        assert_eq!(w.events("assignment.created").await, 0);

        w.go(
            &w.teacher,
            "PATCH",
            &format!("/items/{item}"),
            Some(json!({"publish_at": null})),
        )
        .await;
        assert_eq!(w.tick(Utc::now()).await.announced, 1);
        assert_eq!(w.events("assignment.created").await, 1);
    })
    .await;
}

#[tokio::test]
async fn publishing_the_course_announces_the_assignments_it_was_hiding() {
    db_test(async |pool| {
        let w = world(pool).await;
        let token = &w.teacher.token;
        // A course that is still a draft, with a published module and a published assignment.
        let course = id_of(
            &w.env
                .ok(
                    "POST",
                    token,
                    "/courses",
                    Some(json!({"title": "Draft"})),
                    201,
                )
                .await,
        );
        let module = id_of(
            &w.env
                .ok(
                    "POST",
                    token,
                    &format!("/courses/{course}/modules"),
                    Some(json!({"title": "M"})),
                    201,
                )
                .await,
        );
        w.env
            .ok(
                "POST",
                token,
                &format!("/modules/{module}/publish"),
                Some(json!({"cascade": true})),
                200,
            )
            .await;
        let item = w.draft_in(module, "text", json!({}), json!({})).await;
        w.publish(item).await;
        assert_eq!(w.events("assignment.created").await, 0);
        assert_eq!(w.tick(Utc::now()).await, Tick::default());

        w.go(
            &w.teacher,
            "POST",
            &format!("/courses/{course}/publish"),
            None,
        )
        .await;
        assert_eq!(w.events("assignment.created").await, 1);
        assert_eq!(w.tick(Utc::now()).await.announced, 0);
    })
    .await;
}

#[tokio::test]
async fn publishing_a_module_without_cascade_announces_the_assignments_already_published_in_it() {
    db_test(async |pool| {
        let w = world(pool).await;
        let hidden = id_of(
            &w.env
                .ok(
                    "POST",
                    &w.teacher.token,
                    &format!("/courses/{}/modules", w.course),
                    Some(json!({"title": "Hidden"})),
                    201,
                )
                .await,
        );
        let item = w.draft_in(hidden, "text", json!({}), json!({})).await;
        w.publish(item).await;
        assert_eq!(w.events("assignment.created").await, 0);

        w.go(
            &w.teacher,
            "POST",
            &format!("/modules/{hidden}/publish"),
            Some(json!({"cascade": false})),
        )
        .await;
        assert_eq!(w.events("assignment.created").await, 1);
    })
    .await;
}

// --- submitting a draft at the deadline ----------------------------------------------------------

#[tokio::test]
async fn a_draft_is_submitted_at_the_deadline_and_stamped_with_it() {
    db_test(async |pool| {
        let w = world(pool).await;
        let due = Utc::now() + Duration::hours(1);
        let id = w.published("text", json!({}), auto_submit_at(due)).await;
        w.save(id, essay("my answer")).await;

        // Before the deadline nothing happens.
        assert_eq!(w.tick(due - Duration::minutes(1)).await, Tick::default());
        assert!(w.mine(id).await["draft"].is_object());

        let done = w.tick(due + Duration::seconds(1)).await;
        assert_eq!((done.submitted, done.skipped), (1, 0));
        let mine = w.mine(id).await;
        assert!(mine["draft"].is_null(), "{mine}");
        let attempt = &mine["attempts"][0];
        assert_eq!(attempt["attempt"], 1);
        assert_eq!(attempt["is_late"], false);
        assert_eq!(attempt["content"], essay("my answer"));
        let at: DateTime<Utc> = attempt["submitted_at"].as_str().unwrap().parse().unwrap();
        // The database keeps microseconds.
        assert_eq!(at.timestamp_micros(), due.timestamp_micros());
        assert_eq!(w.events("submission.submitted").await, 1);

        // A second tick finds nothing left to do.
        assert_eq!(w.tick(due + Duration::minutes(1)).await, Tick::default());
        assert_eq!(w.mine(id).await["attempts"].as_array().unwrap().len(), 1);
    })
    .await;
}

#[tokio::test]
async fn keep_draft_leaves_the_draft_alone() {
    db_test(async |pool| {
        let w = world(pool).await;
        let due = Utc::now() + Duration::hours(1);
        let id = w
            .published(
                "text",
                json!({}),
                json!({"due_at": due.to_rfc3339(), "on_deadline": "keep_draft",
                       "accept_late": false}),
            )
            .await;
        w.save(id, essay("unfinished")).await;
        assert_eq!(w.tick(due + Duration::hours(1)).await, Tick::default());
        assert_eq!(w.mine(id).await["draft"]["content"], essay("unfinished"));
    })
    .await;
}

#[tokio::test]
async fn a_draft_the_submit_refuses_is_kept_and_tried_once() {
    db_test(async |pool| {
        let w = world(pool).await;
        let due = Utc::now() + Duration::hours(1);
        let id = w.published("text", json!({}), auto_submit_at(due)).await;
        // A document with no text can be saved but not submitted.
        w.save(id, json!({"v": 1, "attachments": [], "blocks": []}))
            .await;

        let done = w.tick(due + Duration::seconds(1)).await;
        assert_eq!((done.submitted, done.skipped), (0, 1));
        assert!(w.mine(id).await["draft"].is_object());
        assert_eq!(w.events("submission.submitted").await, 0);

        // Marked, so it is not tried on every tick.
        assert_eq!(w.tick(due + Duration::minutes(1)).await, Tick::default());
        let marked: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM assessment.submissions
             WHERE assignment_id = $1 AND auto_submit_skipped_at IS NOT NULL",
        )
        .bind(id)
        .fetch_one(&w.env.pool)
        .await
        .unwrap();
        assert_eq!(marked, 1);
    })
    .await;
}

#[tokio::test]
async fn a_student_who_left_the_course_is_not_submitted_for() {
    db_test(async |pool| {
        let w = world(pool).await;
        let due = Utc::now() + Duration::hours(1);
        let id = w.published("text", json!({}), auto_submit_at(due)).await;
        w.save(id, essay("work")).await;
        w.env
            .ok(
                "DELETE",
                &w.teacher.token,
                &format!("/courses/{}/members/{}", w.course, w.student.id),
                None,
                204,
            )
            .await;

        let done = w.tick(due + Duration::seconds(1)).await;
        assert_eq!((done.submitted, done.skipped), (0, 1));
        let attempts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM assessment.submissions
             WHERE assignment_id = $1 AND status = 'submitted'",
        )
        .bind(id)
        .fetch_one(&w.env.pool)
        .await
        .unwrap();
        assert_eq!(attempts, 0);
    })
    .await;
}

#[tokio::test]
async fn a_draft_started_after_the_deadline_is_the_students_to_finish() {
    db_test(async |pool| {
        let w = world(pool).await;
        let due = Utc::now() + Duration::hours(1);
        let id = w.published("text", json!({}), auto_submit_at(due)).await;
        w.save(id, essay("started late")).await;
        // The draft is made to look as if it began after the deadline.
        sqlx::query("UPDATE assessment.submissions SET created_at = $2 WHERE assignment_id = $1")
            .bind(id)
            .bind(due + Duration::minutes(5))
            .execute(&w.env.pool)
            .await
            .unwrap();

        assert_eq!(w.tick(due + Duration::hours(1)).await, Tick::default());
        assert!(w.mine(id).await["draft"].is_object());
    })
    .await;
}

#[tokio::test]
async fn a_quiz_is_graded_when_it_is_submitted_at_the_deadline() {
    db_test(async |pool| {
        let w = world(pool).await;
        let due = Utc::now() + Duration::hours(1);
        let mut settings = auto_submit_at(due);
        settings["max_points"] = json!("1");
        let id = w.published("quiz", quiz_config(), settings).await;
        w.save(id, json!({"answers": {"q1": "3"}})).await;

        assert_eq!(w.tick(due + Duration::seconds(1)).await.submitted, 1);
        let score: String = sqlx::query_scalar(
            "SELECT draft_score::text FROM gradebook.grades WHERE assignment_id = $1",
        )
        .bind(id)
        .fetch_one(&w.env.pool)
        .await
        .unwrap();
        assert_eq!(score, "1.00");
        assert_eq!(w.events("quiz.attempt_submitted").await, 1);
    })
    .await;
}

#[tokio::test]
async fn a_student_who_submitted_first_is_not_submitted_twice() {
    db_test(async |pool| {
        let w = world(pool).await;
        let due = Utc::now() + Duration::hours(1);
        let id = w.published("text", json!({}), auto_submit_at(due)).await;
        w.save(id, essay("done")).await;
        w.env
            .ok(
                "POST",
                &w.student.token,
                &format!("/assignments/{id}/submission/submit"),
                None,
                201,
            )
            .await;

        assert_eq!(w.tick(due + Duration::seconds(1)).await, Tick::default());
        assert_eq!(w.mine(id).await["attempts"].as_array().unwrap().len(), 1);
    })
    .await;
}
