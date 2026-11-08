//! ADR-0075 and ADR-0076: the events a plugin can subscribe to, from the change that causes each to
//! the delivery that is owed, and the announcements one of them is about. A plugin is owed only
//! what it subscribed to, only for the courses it reaches, and an event holds ids and a time.

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

const ALL_EVENTS: [&str; 9] = [
    "submission.submitted",
    "grade.published",
    "plugin.settings.changed",
    "assignment.created",
    "assignment.updated",
    "assignment.deadline_changed",
    "enrollment.changed",
    "quiz.attempt_submitted",
    "announcement.published",
];

struct World {
    env: Env,
    state: AppState,
    teacher: Account,
    student: Account,
    other_student: Account,
    admin: Account,
    course: Uuid,
    module: Uuid,
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
    let student = env.user("student@x.edu", Role::Student).await;
    let other_student = env.user("other@x.edu", Role::Student).await;
    let admin = env.user("admin@x.edu", Role::Admin).await;
    let (course, module) = env.course(&teacher, &[&student]).await;
    World {
        env,
        state,
        teacher,
        student,
        other_student,
        admin,
        course,
        module,
    }
}

fn doc(text: &str) -> Value {
    json!({"v": 1, "attachments": [], "blocks": [
        {"id": "p1", "type": "paragraph", "content": [{"type": "text", "text": text}]}]})
}

fn quiz_config() -> Value {
    json!({"v": 1, "shuffle_questions": false, "show_correct": false, "questions": [
        {"id": "q1", "type": "number", "points": "1", "prompt": doc("one"),
         "answer": "3", "tolerance": "0"}]})
}

impl World {
    /// A plugin that subscribed to every event.
    async fn plugin(&self, id: &str, subscribes: &[&str], courses: CourseScope) -> Installed {
        let manifest = parse_manifest(
            json!({"id": id, "version": "1", "api": "1", "subscribes": subscribes,
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
            &Scope::ALL,
            &courses,
        )
        .await
        .unwrap()
    }

    /// What the core does by itself until it has nothing left to do: the outbox is run until empty,
    /// since one handler (a group's members) writes events of its own.
    async fn pump(&self) {
        for _ in 0..5 {
            let done = events::process_once(&self.state.db, &self.state.event_handlers())
                .await
                .expect("the outbox");
            if done == 0 {
                return;
            }
        }
    }

    /// The events owed to a plugin, oldest first: `(type, data, occurred_at)`.
    async fn owed(&self, plugin: &str) -> Vec<(String, Value, chrono::DateTime<chrono::Utc>)> {
        self.pump().await;
        sqlx::query_as(
            "SELECT d.event_type, d.data, d.occurred_at FROM plugins.deliveries d
             JOIN plugins.plugin_installs i ON i.id = d.install_id
             WHERE i.plugin_id = $1 ORDER BY d.id",
        )
        .bind(plugin)
        .fetch_all(&self.env.pool)
        .await
        .unwrap()
    }

    async fn kinds(&self, plugin: &str) -> Vec<String> {
        self.owed(plugin).await.into_iter().map(|e| e.0).collect()
    }

    async fn forget(&self) {
        self.pump().await;
        sqlx::query("DELETE FROM plugins.deliveries")
            .execute(&self.env.pool)
            .await
            .unwrap();
    }

    async fn call(
        &self,
        who: &Account,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> common::Resp {
        self.env.call(method, &who.token, path, body).await
    }

    async fn item(&self, module: Uuid, kind: &str, config: Value, points: &str) -> Uuid {
        let body = json!({"kind": "assignment", "title": "T",
            "assignment": {"type": kind, "config": config, "max_points": points}});
        id_of(
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
        )
    }

    async fn publish(&self, item: Uuid) {
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

    async fn patch(&self, item: Uuid, body: Value) {
        self.env
            .ok(
                "PATCH",
                &self.teacher.token,
                &format!("/items/{item}"),
                Some(body),
                200,
            )
            .await;
    }

    async fn take(&self, who: &Account, assignment: Uuid, content: Value) {
        let path = format!("/assignments/{assignment}/submission");
        self.env
            .ok(
                "PUT",
                &who.token,
                &format!("{path}/draft"),
                Some(json!({"content": content})),
                200,
            )
            .await;
        self.env
            .ok("POST", &who.token, &format!("{path}/submit"), None, 201)
            .await;
    }

    async fn group(&self, name: &str) -> Uuid {
        id_of(
            &self
                .env
                .ok(
                    "POST",
                    &self.admin.token,
                    "/groups",
                    Some(json!({"name": name})),
                    201,
                )
                .await,
        )
    }
}

fn data(events: &[(String, Value, chrono::DateTime<chrono::Utc>)], kind: &str) -> Vec<Value> {
    events
        .iter()
        .filter(|e| e.0 == kind)
        .map(|e| e.1.clone())
        .collect()
}

// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_plugin_is_owed_only_the_events_it_subscribed_to_and_each_carries_when_it_happened() {
    db_test(async |pool| {
        let w = world(pool).await;
        w.plugin("everything", &ALL_EVENTS, CourseScope::All).await;
        w.plugin(
            "deadlines",
            &["assignment.deadline_changed"],
            CourseScope::All,
        )
        .await;
        w.plugin("silent", &[], CourseScope::All).await;
        w.forget().await;

        let before = chrono::Utc::now() - chrono::TimeDelta::seconds(5);
        let a = w.item(w.module, "text", json!({}), "10").await;
        w.publish(a).await;
        w.patch(
            a,
            json!({"due_at": "2099-01-01T10:00:00Z", "on_deadline": "keep_draft",
                   "accept_late": false}),
        )
        .await;

        let all = w.owed("everything").await;
        let kinds: Vec<_> = all.iter().map(|e| e.0.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "assignment.created",
                "assignment.updated",
                "assignment.deadline_changed"
            ]
        );
        for (_, payload, at) in &all {
            // Ids only, and the time next to them.
            assert_eq!(payload["course_id"], json!(w.course));
            assert_eq!(payload["assignment_id"], json!(a));
            assert_eq!(payload.as_object().unwrap().len(), 2, "{payload}");
            assert!(*at > before && *at <= chrono::Utc::now(), "{at}");
        }
        assert_eq!(w.kinds("deadlines").await, ["assignment.deadline_changed"]);
        assert!(w.kinds("silent").await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn an_assignment_is_news_when_students_can_see_it_and_when_what_they_see_changes() {
    db_test(async |pool| {
        let w = world(pool).await;
        w.plugin("p", &ALL_EVENTS, CourseScope::All).await;
        w.forget().await;

        // A draft: created and changed in private, nothing is said.
        let draft = w.item(w.module, "text", json!({}), "10").await;
        w.patch(draft, json!({"max_points": "12"})).await;
        assert!(w.kinds("p").await.is_empty());

        // Publishing makes it news, once; publishing it again is not a second creation.
        w.publish(draft).await;
        w.publish(draft).await;
        assert_eq!(w.kinds("p").await, ["assignment.created"]);

        // A published assignment: points are an update; a deadline is an update and a deadline
        // change; the same deadline again is an update and no change; a title edit that students
        // do not see yet is nothing.
        w.forget().await;
        w.patch(draft, json!({"max_points": "11"})).await;
        assert_eq!(w.kinds("p").await, ["assignment.updated"]);
        w.forget().await;
        let due = json!({"due_at": "2099-01-01T10:00:00Z", "on_deadline": "keep_draft",
                         "accept_late": false});
        w.patch(draft, due.clone()).await;
        assert_eq!(
            w.kinds("p").await,
            ["assignment.updated", "assignment.deadline_changed"]
        );
        w.forget().await;
        w.patch(draft, due).await;
        assert_eq!(w.kinds("p").await, ["assignment.updated"]);
        w.forget().await;
        w.patch(draft, json!({"title": "Only the working copy"}))
            .await;
        assert!(w.kinds("p").await.is_empty());
        // Publishing that revision is what students see: an update.
        w.env
            .ok(
                "POST",
                &w.teacher.token,
                &format!("/items/{draft}/revision/publish"),
                None,
                200,
            )
            .await;
        assert_eq!(w.kinds("p").await, ["assignment.updated"]);

        // Clearing the deadline is a change as well.
        w.forget().await;
        w.patch(
            draft,
            json!({"due_at": null, "on_deadline": null, "accept_late": null}),
        )
        .await;
        assert_eq!(
            w.kinds("p").await,
            ["assignment.updated", "assignment.deadline_changed"]
        );
    })
    .await;
}

#[tokio::test]
async fn publishing_a_module_announces_its_assignments_and_not_its_lectures() {
    db_test(async |pool| {
        let w = world(pool).await;
        w.plugin("p", &ALL_EVENTS, CourseScope::All).await;
        let module = id_of(
            &w.env
                .ok(
                    "POST",
                    &w.teacher.token,
                    &format!("/courses/{}/modules", w.course),
                    Some(json!({"title": "Draft module"})),
                    201,
                )
                .await,
        );
        let one = w.item(module, "text", json!({}), "10").await;
        let two = w.item(module, "text", json!({}), "5").await;
        w.env
            .ok(
                "POST",
                &w.teacher.token,
                &format!("/modules/{module}/items"),
                Some(json!({"kind": "text", "title": "A lecture"})),
                201,
            )
            .await;
        w.forget().await;

        w.env
            .ok(
                "POST",
                &w.teacher.token,
                &format!("/modules/{module}/publish"),
                Some(json!({"cascade": true})),
                200,
            )
            .await;
        let events = w.owed("p").await;
        let mut announced: Vec<String> = data(&events, "assignment.created")
            .iter()
            .map(|d| d["assignment_id"].as_str().unwrap().to_owned())
            .collect();
        announced.sort();
        let mut expected = vec![one.to_string(), two.to_string()];
        expected.sort();
        assert_eq!(announced, expected);
        assert_eq!(events.len(), 2, "the lecture says nothing");
    })
    .await;
}

#[tokio::test]
async fn a_quiz_attempt_is_announced_with_its_submission_and_an_essay_is_not_a_quiz() {
    db_test(async |pool| {
        let w = world(pool).await;
        w.plugin("p", &ALL_EVENTS, CourseScope::All).await;
        let quiz = w.item(w.module, "quiz", quiz_config(), "1").await;
        w.publish(quiz).await;
        let essay = w.item(w.module, "text", json!({}), "10").await;
        w.publish(essay).await;
        w.forget().await;

        w.take(&w.student, essay, common::essay("words")).await;
        assert_eq!(w.kinds("p").await, ["submission.submitted"]);
        w.forget().await;

        w.take(&w.student, quiz, json!({"answers": {"q1": "3"}}))
            .await;
        let events = w.owed("p").await;
        let kinds: Vec<_> = events.iter().map(|e| e.0.as_str()).collect();
        assert_eq!(kinds, ["submission.submitted", "quiz.attempt_submitted"]);
        let attempt = &data(&events, "quiz.attempt_submitted")[0];
        assert_eq!(attempt["assignment_id"], json!(quiz));
        assert_eq!(attempt["student_id"], json!(w.student.id));
        assert_eq!(attempt["course_id"], json!(w.course));
        assert_eq!(
            attempt["submission_id"],
            data(&events, "submission.submitted")[0]["submission_id"]
        );
        // Scores are not in it.
        assert_eq!(attempt.as_object().unwrap().len(), 4);
    })
    .await;
}

#[tokio::test]
async fn the_roster_changes_of_a_course_are_announced_one_person_at_a_time() {
    db_test(async |pool| {
        let w = world(pool).await;
        w.plugin("p", &ALL_EVENTS, CourseScope::All).await;
        w.forget().await;
        let path = |user: Uuid| format!("/courses/{}/members/{user}", w.course);

        // Adding, changing the role of, and removing a member.
        w.env
            .ok(
                "PUT",
                &w.teacher.token,
                &path(w.other_student.id),
                Some(json!({"role": "student"})),
                200,
            )
            .await;
        let events = w.owed("p").await;
        assert_eq!(
            data(&events, "enrollment.changed"),
            [json!({"course_id": w.course, "user_id": w.other_student.id})]
        );
        // The same role again changes nothing, so it says nothing.
        w.forget().await;
        w.env
            .ok(
                "PUT",
                &w.teacher.token,
                &path(w.other_student.id),
                Some(json!({"role": "student"})),
                200,
            )
            .await;
        assert!(w.kinds("p").await.is_empty());
        w.env
            .ok(
                "DELETE",
                &w.teacher.token,
                &path(w.other_student.id),
                None,
                204,
            )
            .await;
        assert_eq!(w.kinds("p").await, ["enrollment.changed"]);
    })
    .await;
}

/// The courses an `enrollment.changed` of `user` names, sorted.
fn courses_of(
    events: &[(String, Value, chrono::DateTime<chrono::Utc>)],
    user: Uuid,
) -> Vec<String> {
    let mut courses: Vec<String> = data(events, "enrollment.changed")
        .iter()
        .filter(|d| d["user_id"] == json!(user))
        .map(|d| d["course_id"].as_str().unwrap().to_owned())
        .collect();
    courses.sort();
    courses
}

#[tokio::test]
async fn a_change_of_a_users_status_or_role_is_announced_for_every_roster_they_are_on() {
    db_test(async |pool| {
        let w = world(pool).await;
        w.plugin("p", &ALL_EVENTS, CourseScope::All).await;
        // The student is a member of two courses; another student is on the first only through a
        // group; a second teacher teaches the first.
        let (second, _) = w.env.course(&w.teacher, &[&w.student]).await;
        let group = w.group("7-A").await;
        w.env
            .ok(
                "POST",
                &w.admin.token,
                &format!("/groups/{group}/members"),
                Some(json!({"user_ids": [w.other_student.id]})),
                200,
            )
            .await;
        w.env
            .ok(
                "POST",
                &w.teacher.token,
                &format!("/courses/{}/groups", w.course),
                Some(json!({"group_id": group, "subgroup_id": null})),
                201,
            )
            .await;
        let colleague = w.env.user("colleague@x.edu", Role::Teacher).await;
        w.env
            .ok(
                "PUT",
                &w.teacher.token,
                &format!("/courses/{}/members/{}", w.course, colleague.id),
                Some(json!({"role": "teacher"})),
                200,
            )
            .await;
        let both = {
            let mut both = vec![w.course.to_string(), second.to_string()];
            both.sort();
            both
        };
        let patch = async |user: Uuid, body: Value| {
            w.env
                .ok(
                    "PATCH",
                    &w.admin.token,
                    &format!("/admin/users/{user}"),
                    Some(body),
                    200,
                )
                .await;
        };
        w.forget().await;

        // A new name moves nobody, and neither does saying what is already so.
        patch(w.student.id, json!({"name": "Anna"})).await;
        patch(w.student.id, json!({"status": "active"})).await;
        assert!(w.kinds("p").await.is_empty());

        // A disabled student leaves every roster; an enabled one comes back to each.
        patch(w.student.id, json!({"status": "disabled"})).await;
        assert_eq!(courses_of(&w.owed("p").await, w.student.id), both);
        w.forget().await;
        patch(w.student.id, json!({"status": "active"})).await;
        assert_eq!(courses_of(&w.owed("p").await, w.student.id), both);
        w.forget().await;

        // The same for a student who is on a roster through a group alone.
        patch(w.other_student.id, json!({"status": "disabled"})).await;
        let events = w.owed("p").await;
        assert_eq!(
            courses_of(&events, w.other_student.id),
            [w.course.to_string()]
        );
        // Only people of the change, and ids only.
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1.as_object().unwrap().len(), 2);
        w.forget().await;

        // A group brings students only: a student who becomes a teacher leaves it.
        patch(w.other_student.id, json!({"status": "active"})).await;
        w.forget().await;
        patch(w.other_student.id, json!({"role": "teacher"})).await;
        assert_eq!(
            courses_of(&w.owed("p").await, w.other_student.id),
            [w.course.to_string()]
        );
        w.forget().await;

        // A teacher who becomes a student stops teaching: the membership that goes is announced once,
        // not twice.
        patch(colleague.id, json!({"role": "student"})).await;
        let events = w.owed("p").await;
        assert_eq!(courses_of(&events, colleague.id), [w.course.to_string()]);
        assert_eq!(events.len(), 1, "{events:?}");
    })
    .await;
}

#[tokio::test]
async fn a_linked_group_announces_its_students_when_linked_unlinked_and_when_it_changes() {
    db_test(async |pool| {
        let w = world(pool).await;
        w.plugin("p", &ALL_EVENTS, CourseScope::All).await;
        let group = w.group("7-A").await;
        let members = format!("/groups/{group}/members");
        w.env
            .ok(
                "POST",
                &w.admin.token,
                &members,
                Some(json!({"user_ids": [w.other_student.id]})),
                200,
            )
            .await;
        w.forget().await;

        // Linking puts the group's students on the roster.
        let link = id_of(
            &w.env
                .ok(
                    "POST",
                    &w.teacher.token,
                    &format!("/courses/{}/groups", w.course),
                    Some(json!({"group_id": group, "subgroup_id": null})),
                    201,
                )
                .await,
        );
        assert_eq!(
            data(&w.owed("p").await, "enrollment.changed"),
            [json!({"course_id": w.course, "user_id": w.other_student.id})]
        );

        // A student who joins the group joins the course: `org` knows the group, `learning`
        // knows the course, and the plugin hears of it as a roster change.
        w.forget().await;
        let newcomer = w.env.user("newcomer@x.edu", Role::Student).await;
        w.env
            .ok(
                "POST",
                &w.admin.token,
                &members,
                Some(json!({"user_ids": [newcomer.id]})),
                200,
            )
            .await;
        let events = w.owed("p").await;
        assert_eq!(
            data(&events, "enrollment.changed"),
            [json!({"course_id": w.course, "user_id": newcomer.id})]
        );
        // The step in between is internal: it is not an event a plugin can ask for.
        assert!(events.iter().all(|e| e.0 != "group.members_changed"));

        // Leaving the group is the same.
        w.forget().await;
        w.env
            .ok(
                "DELETE",
                &w.admin.token,
                &format!("/groups/{group}/members/{}", newcomer.id),
                None,
                204,
            )
            .await;
        assert_eq!(w.kinds("p").await, ["enrollment.changed"]);

        // Unlinking takes the students off the roster.
        w.forget().await;
        w.env
            .ok(
                "DELETE",
                &w.teacher.token,
                &format!("/courses/{}/groups/{link}", w.course),
                None,
                204,
            )
            .await;
        assert_eq!(
            data(&w.owed("p").await, "enrollment.changed"),
            [json!({"course_id": w.course, "user_id": w.other_student.id})]
        );

        // A group no course uses tells nobody.
        w.forget().await;
        w.env
            .ok(
                "POST",
                &w.admin.token,
                &members,
                Some(json!({"user_ids": [w.student.id]})),
                200,
            )
            .await;
        assert!(w.kinds("p").await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn a_plugin_is_sent_the_events_of_its_courses_and_no_others() {
    db_test(async |pool| {
        let w = world(pool).await;
        let (other_course, other_module) = w.env.course(&w.teacher, &[&w.student]).await;
        w.plugin("here", &ALL_EVENTS, CourseScope::Only(vec![w.course]))
            .await;
        w.plugin("there", &ALL_EVENTS, CourseScope::Only(vec![other_course]))
            .await;
        w.plugin("nowhere", &ALL_EVENTS, CourseScope::Only(Vec::new()))
            .await;
        w.plugin("everywhere", &ALL_EVENTS, CourseScope::All).await;
        w.forget().await;

        let mine = w.item(w.module, "text", json!({}), "10").await;
        let theirs = w.item(other_module, "text", json!({}), "10").await;
        w.publish(mine).await;
        w.publish(theirs).await;

        let about = |events: Vec<(String, Value, chrono::DateTime<chrono::Utc>)>| -> Vec<String> {
            events
                .iter()
                .map(|e| e.1["assignment_id"].as_str().unwrap().to_owned())
                .collect()
        };
        assert_eq!(about(w.owed("here").await), [mine.to_string()]);
        assert_eq!(about(w.owed("there").await), [theirs.to_string()]);
        assert!(w.owed("nowhere").await.is_empty());
        assert_eq!(w.owed("everywhere").await.len(), 2);
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// Announcements (ADR-0076)
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_teacher_publishes_an_announcement_the_course_reads_and_the_plugins_hear_of() {
    db_test(async |pool| {
        let w = world(pool).await;
        w.plugin("p", &ALL_EVENTS, CourseScope::All).await;
        w.forget().await;
        let path = format!("/courses/{}/announcements", w.course);

        // A student cannot write; nothing is sent.
        w.call(
            &w.student,
            "POST",
            &path,
            Some(json!({"title": "Hi", "body": "Hello"})),
        )
        .await
        .problem(403);
        // Invalid text is a 422: blank, too long.
        for (title, body) in [("  ", "x"), ("t", "  "), ("", "x"), ("t", "")] {
            w.call(
                &w.teacher,
                "POST",
                &path,
                Some(json!({"title": title, "body": body})),
            )
            .await
            .problem(422);
        }
        w.call(
            &w.teacher,
            "POST",
            &path,
            Some(json!({"title": "t".repeat(201), "body": "x"})),
        )
        .await
        .problem(422);
        assert!(w.kinds("p").await.is_empty());

        let created = w
            .env
            .ok(
                "POST",
                &w.teacher.token,
                &path,
                Some(json!({"title": "  Exam moved  ", "body": "Friday, room 4"})),
                201,
            )
            .await;
        let id = id_of(&created);
        assert_eq!(created["title"], "Exam moved");
        assert_eq!(created["author_id"], json!(w.teacher.id));
        assert_eq!(
            data(&w.owed("p").await, "announcement.published"),
            [json!({"course_id": w.course, "announcement_id": id})]
        );

        // The course reads it, newest first; a student of another course and a stranger do not.
        let second = id_of(
            &w.env
                .ok(
                    "POST",
                    &w.teacher.token,
                    &path,
                    Some(json!({"title": "Second", "body": "again"})),
                    201,
                )
                .await,
        );
        let read = w.env.ok("GET", &w.student.token, &path, None, 200).await;
        let ids: Vec<_> = read["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(ids, [second.to_string(), id.to_string()]);
        let page = w
            .env
            .ok(
                "GET",
                &w.student.token,
                &format!("{path}?limit=1"),
                None,
                200,
            )
            .await;
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
        let next = page["next_cursor"].as_str().unwrap();
        let rest = w
            .env
            .ok(
                "GET",
                &w.student.token,
                &format!("{path}?limit=1&cursor={next}"),
                None,
                200,
            )
            .await;
        assert_eq!(rest["items"][0]["id"], json!(id));
        w.call(&w.other_student, "GET", &path, None)
            .await
            .problem(404);

        // An edit is not news; a student cannot edit or delete; the author can delete.
        w.forget().await;
        let one = format!("/announcements/{id}");
        let edited = w
            .env
            .ok(
                "PATCH",
                &w.teacher.token,
                &one,
                Some(json!({"body": "Friday, room 5"})),
                200,
            )
            .await;
        assert_eq!(edited["body"], "Friday, room 5");
        assert_eq!(edited["title"], "Exam moved");
        assert!(w.kinds("p").await.is_empty());
        w.call(&w.student, "PATCH", &one, Some(json!({"body": "x"})))
            .await
            .problem(403);
        w.call(&w.student, "DELETE", &one, None).await.problem(403);
        w.call(&w.other_student, "PATCH", &one, Some(json!({"body": "x"})))
            .await
            .problem(404);
        w.env.ok("DELETE", &w.teacher.token, &one, None, 204).await;
        w.call(&w.teacher, "DELETE", &one, None).await.problem(404);
    })
    .await;
}

#[tokio::test]
async fn a_plugin_reads_the_announcements_of_the_courses_it_reaches() {
    db_test(async |pool| {
        let w = world(pool).await;
        let (other_course, _) = w.env.course(&w.teacher, &[&w.student]).await;
        let all = w.plugin("all", &[], CourseScope::All).await;
        let only = w
            .plugin("only", &[], CourseScope::Only(vec![w.course]))
            .await;
        let none = {
            let manifest = parse_manifest(
                json!({"id": "bare", "version": "1", "api": "1", "scopes": ["announcements:read"]})
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
                &CourseScope::All,
            )
            .await
            .unwrap()
        };
        for (course, title) in [(w.course, "Mine"), (other_course, "Theirs")] {
            w.env
                .ok(
                    "POST",
                    &w.teacher.token,
                    &format!("/courses/{course}/announcements"),
                    Some(json!({"title": title, "body": "text"})),
                    201,
                )
                .await;
        }
        let get = async |who: &Installed, query: &str| {
            common::Http::new(&w.env.base)
                .get(&format!("/plugin-api/announcements{query}"))
                .bearer(who.token.expose())
                .send()
                .await
        };
        let titles = |response: &common::Resp| -> Vec<String> {
            response.json()["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a["title"].as_str().unwrap().to_owned())
                .collect()
        };

        let everything = get(&all, "").await;
        assert_eq!(everything.status, 200);
        assert_eq!(everything.header("cache-control"), "no-store");
        assert_eq!(titles(&everything), ["Mine", "Theirs"]);
        assert_eq!(titles(&get(&only, "").await), ["Mine"]);
        assert_eq!(
            titles(&get(&all, &format!("?course_id={other_course}")).await),
            ["Theirs"]
        );
        get(&only, &format!("?course_id={other_course}"))
            .await
            .problem(403);
        get(&all, &format!("?course_id={}", Uuid::now_v7()))
            .await
            .problem(404);
        // Without the scope: 403, whoever it asks for.
        let denied = {
            let manifest = parse_manifest(
                json!({"id": "other", "version": "1", "api": "1", "scopes": ["grades:read"]})
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
                &[Scope::GradesRead],
                &CourseScope::All,
            )
            .await
            .unwrap()
        };
        get(&denied, "").await.problem(403);
        get(&none, "").await.problem(403);
        // Paging and catching up.
        let page = get(&all, "?limit=1").await;
        let cursor = page.json()["next_cursor"].as_str().unwrap().to_owned();
        assert_eq!(
            titles(&get(&all, &format!("?limit=1&cursor={cursor}")).await),
            ["Theirs"]
        );
        let since = (chrono::Utc::now() + chrono::TimeDelta::hours(1))
            .to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
        assert!(titles(&get(&all, &format!("?updated_since={since}")).await).is_empty());
    })
    .await;
}
