//! ADR-0048: the course header, archiving, deletion, templates, people and candidates.

mod common;

use common::{Account, Env, db_test, essay, id_of};
use serde_json::{Value, json};
use server::identity::Role;
use uuid::Uuid;

struct World {
    env: Env,
    admin: Account,
    owner: Account,
    assistant: Account,
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
    let assistant = env.user("assistant@lms.test", Role::Teacher).await;
    let student = env.user("student@lms.test", Role::Student).await;
    let other = env.user("other@lms.test", Role::Student).await;
    let stranger = env.user("stranger@lms.test", Role::Teacher).await;
    let (course, module) = env.course(&owner, &[&student, &other]).await;
    env.ok(
        "PUT",
        &owner.token,
        &format!("/courses/{course}/members/{}", assistant.id),
        Some(json!({"role": "assistant"})),
        200,
    )
    .await;
    World {
        env,
        admin,
        owner,
        assistant,
        student,
        other,
        stranger,
        course,
        module,
    }
}

impl World {
    async fn get(&self, who: &Account, path: &str) -> Value {
        self.env.ok("GET", &who.token, path, None, 200).await
    }

    async fn status_of(&self, method: &str, who: &Account, path: &str, body: Option<Value>) -> u16 {
        self.env.call(method, &who.token, path, body).await.status
    }

    /// An assignment in `module`: published unless `draft`.
    async fn assignment(&self, points: &str, bonus: bool, draft: bool) -> Uuid {
        let body = json!({"kind": "assignment", "title": "A",
            "assignment": {"type": "text", "max_points": points, "is_bonus": bonus}});
        let item = id_of(
            &self
                .env
                .ok(
                    "POST",
                    &self.owner.token,
                    &format!("/modules/{}/items", self.module),
                    Some(body),
                    201,
                )
                .await,
        );
        if !draft {
            self.env
                .ok(
                    "POST",
                    &self.owner.token,
                    &format!("/items/{item}/publish"),
                    None,
                    200,
                )
                .await;
        }
        item
    }

    async fn submit(&self, who: &Account, assignment: Uuid) {
        let base = format!("/assignments/{assignment}/submission");
        self.env
            .ok(
                "PUT",
                &who.token,
                &format!("{base}/draft"),
                Some(json!({"content": essay("my answer")})),
                200,
            )
            .await;
        self.env
            .ok("POST", &who.token, &format!("{base}/submit"), None, 201)
            .await;
    }

    async fn grade(&self, assignment: Uuid, student: &Account, score: &str) {
        self.env
            .ok(
                "PUT",
                &self.owner.token,
                &format!("/assignments/{assignment}/grades/{}", student.id),
                Some(json!({"score": score, "feedback": null})),
                200,
            )
            .await;
    }

    async fn archive(&self) {
        self.env
            .ok(
                "POST",
                &self.owner.token,
                &format!("/courses/{}/archive", self.course),
                None,
                200,
            )
            .await;
    }

    async fn audit_actions(&self) -> Vec<String> {
        sqlx::query_scalar!("SELECT action FROM events.audit_log ORDER BY at, id")
            .fetch_all(&self.env.pool)
            .await
            .unwrap()
    }
}

fn names(list: &Value) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|p| {
            format!(
                "{} {}",
                p["name"].as_str().unwrap(),
                p["surname"].as_str().unwrap()
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Info
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_header_counts_the_people_and_adds_up_only_the_points_a_reader_may_see() {
    db_test(async |pool| {
        let w = world(pool).await;
        let term = w
            .env
            .ok(
                "POST",
                &w.admin.token,
                "/terms",
                Some(json!({"name": "Fall 2026", "starts_on": "2026-09-01", "ends_on": "2027-01-31"})),
                201,
            )
            .await;
        w.env
            .ok(
                "PATCH",
                &w.owner.token,
                &format!("/courses/{}", w.course),
                Some(json!({"term_id": term["id"]})),
                200,
            )
            .await;
        w.assignment("20", false, false).await;
        w.assignment("5", true, false).await; // extra credit is not counted
        w.assignment("10", false, true).await; // a draft is only counted for staff

        let path = format!("/courses/{}/info", w.course);
        let staff = w.get(&w.owner, &path).await;
        assert_eq!(staff["course_id"], w.course.to_string());
        assert_eq!(staff["term_name"], "Fall 2026");
        assert_eq!(staff["owner_email"], "owner@lms.test");
        assert_eq!(
            (staff["student_count"].as_u64(), staff["assistant_count"].as_u64(), staff["former_count"].as_u64()),
            (Some(2), Some(1), Some(0))
        );
        assert_eq!(staff["total_points"], "30.00");
        assert_eq!(staff["graded_items"], 2);
        assert_eq!(staff["has_grades"], false);
        assert_eq!((staff["code"].as_str(), staff["description"].as_str()), (Some(""), Some("")));
        assert!(staff["content_updated_at"].is_string());

        let student = w.get(&w.student, &path).await;
        assert_eq!(student["total_points"], "20.00");
        assert_eq!(student["graded_items"], 1);

        w.env.ok("GET", &w.stranger.token, &path, None, 404).await;
        w.env
            .ok("GET", &w.owner.token, &format!("/courses/{}/info", Uuid::now_v7()), None, 404)
            .await;
    })
    .await;
}

#[tokio::test]
async fn the_header_knows_when_grades_exist_and_who_left_the_roster_with_results() {
    db_test(async |pool| {
        let w = world(pool).await;
        let assignment = w.assignment("20", false, false).await;
        let path = format!("/courses/{}/info", w.course);
        assert_eq!(w.get(&w.owner, &path).await["has_grades"], false);

        w.submit(&w.student, assignment).await;
        w.grade(assignment, &w.student, "15").await;
        assert_eq!(w.get(&w.owner, &path).await["has_grades"], true);

        // The student leaves the roster; the results stay, and the student is a former one.
        w.env
            .ok(
                "DELETE",
                &w.owner.token,
                &format!("/courses/{}/members/{}", w.course, w.student.id),
                None,
                204,
            )
            .await;
        let info = w.get(&w.owner, &path).await;
        assert_eq!(info["student_count"], 1);
        assert_eq!(info["former_count"], 1);
    })
    .await;
}

#[tokio::test]
async fn code_and_description_are_edited_by_staff_trimmed_and_bounded_and_audited_for_admins() {
    db_test(async |pool| {
        let w = world(pool).await;
        let path = format!("/courses/{}/info", w.course);

        let saved = w
            .env
            .ok(
                "PATCH",
                &w.assistant.token,
                &path,
                Some(json!({"code": "  CS 305 ", "description": "Algorithms\nand data"})),
                200,
            )
            .await;
        assert_eq!(saved["code"], "CS 305");
        assert_eq!(saved["description"], "Algorithms\nand data");
        // Only what is sent changes, and an empty value clears.
        let cleared = w
            .env
            .ok(
                "PATCH",
                &w.owner.token,
                &path,
                Some(json!({"description": ""})),
                200,
            )
            .await;
        assert_eq!(
            (cleared["code"].as_str(), cleared["description"].as_str()),
            (Some("CS 305"), Some(""))
        );

        for bad in [
            json!({"code": "x".repeat(33)}),
            json!({"description": "y".repeat(301)}),
            json!({"code": "a\tb"}),
        ] {
            let response = w.env.call("PATCH", &w.owner.token, &path, Some(bad)).await;
            let problem = response.problem(422);
            assert_eq!(problem.errors.len(), 1, "{problem:?}");
        }
        assert_eq!(
            w.status_of("PATCH", &w.student, &path, Some(json!({"code": "X"})))
                .await,
            403
        );
        assert_eq!(
            w.status_of("PATCH", &w.stranger, &path, Some(json!({"code": "X"})))
                .await,
            404
        );

        // Teachers' own edits are not audited; an admin's are.
        assert!(w.audit_actions().await.iter().all(|a| a != "course.update"));
        w.env
            .ok(
                "PATCH",
                &w.admin.token,
                &path,
                Some(json!({"code": "ADM"})),
                200,
            )
            .await;
        assert!(
            w.audit_actions()
                .await
                .contains(&"course.update".to_owned())
        );
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// Archive and restore
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn archiving_is_a_root_action_that_keeps_the_course_readable_and_restoring_publishes_it() {
    db_test(async |pool| {
        let w = world(pool).await;
        let course = format!("/courses/{}", w.course);
        assert_eq!(
            w.status_of("POST", &w.assistant, &format!("{course}/archive"), None)
                .await,
            403
        );
        assert_eq!(
            w.status_of("POST", &w.student, &format!("{course}/archive"), None)
                .await,
            403
        );
        assert_eq!(
            w.status_of("POST", &w.stranger, &format!("{course}/archive"), None)
                .await,
            404
        );

        let archived = w
            .env
            .ok(
                "POST",
                &w.owner.token,
                &format!("{course}/archive"),
                None,
                200,
            )
            .await;
        assert_eq!(archived["status"], "archived");
        // Idempotent.
        let again = w
            .env
            .ok(
                "POST",
                &w.owner.token,
                &format!("{course}/archive"),
                None,
                200,
            )
            .await;
        assert_eq!(again["status"], "archived");

        // Everyone who could read it still reads it.
        for who in [&w.student, &w.assistant, &w.owner] {
            assert_eq!(w.get(who, &course).await["status"], "archived");
            w.get(who, &format!("/courses/{}/content", w.course)).await;
        }
        let listed = w.get(&w.student, "/me/courses").await;
        assert_eq!(listed["items"][0]["id"], w.course.to_string());

        // Restoring is a root action too.
        assert_eq!(
            w.status_of("POST", &w.assistant, &format!("{course}/restore"), None)
                .await,
            403
        );
        let restored = w
            .env
            .ok(
                "POST",
                &w.owner.token,
                &format!("{course}/restore"),
                None,
                200,
            )
            .await;
        assert_eq!(restored["status"], "published");
        let noop = w
            .env
            .ok(
                "POST",
                &w.owner.token,
                &format!("{course}/restore"),
                None,
                200,
            )
            .await;
        assert_eq!(noop["status"], "published");
    })
    .await;
}

#[tokio::test]
async fn an_admin_archiving_and_restoring_is_audited_and_a_draft_comes_back_published() {
    db_test(async |pool| {
        let w = world(pool).await;
        let draft = common::create_course(&w.env.http, &w.owner.token, "Draft").await;
        let path = format!("/courses/{draft}");

        let archived = w
            .env
            .ok(
                "POST",
                &w.admin.token,
                &format!("{path}/archive"),
                None,
                200,
            )
            .await;
        assert_eq!(archived["status"], "archived");
        let restored = w
            .env
            .ok(
                "POST",
                &w.admin.token,
                &format!("{path}/restore"),
                None,
                200,
            )
            .await;
        assert_eq!(restored["status"], "published");

        let actions = w.audit_actions().await;
        assert!(
            actions.contains(&"course.archive".to_owned()),
            "{actions:?}"
        );
        assert!(
            actions.contains(&"course.restore".to_owned()),
            "{actions:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn an_archived_course_refuses_every_write_with_409_and_keeps_every_read() {
    db_test(async |pool| {
        let w = world(pool).await;
        let assignment = w.assignment("20", false, false).await;
        w.submit(&w.student, assignment).await;
        w.grade(assignment, &w.student, "10").await;
        let course = format!("/courses/{}", w.course);
        w.archive().await;

        let refused: Vec<(&str, &Account, String, Option<Value>)> = vec![
            (
                "PATCH",
                &w.owner,
                course.clone(),
                Some(json!({"title": "New"})),
            ),
            (
                "PATCH",
                &w.owner,
                format!("{course}/info"),
                Some(json!({"code": "X"})),
            ),
            (
                "PUT",
                &w.owner,
                format!("{course}/members/{}", w.stranger.id),
                Some(json!({"role": "assistant"})),
            ),
            (
                "DELETE",
                &w.owner,
                format!("{course}/members/{}", w.other.id),
                None,
            ),
            (
                "PUT",
                &w.owner,
                format!("/assignments/{assignment}/grades/{}", w.student.id),
                Some(json!({"score": "12", "feedback": null})),
            ),
            (
                "POST",
                &w.owner,
                format!("/assignments/{assignment}/grades/publish"),
                Some(json!({"students": "all"})),
            ),
            (
                "PUT",
                &w.student,
                format!("/assignments/{assignment}/submission/draft"),
                Some(json!({"content": essay("late")})),
            ),
            (
                "POST",
                &w.owner,
                format!("{course}/modules"),
                Some(json!({"title": "N"})),
            ),
        ];
        for (method, who, path, body) in refused {
            let response = w.env.call(method, &who.token, &path, body).await;
            let problem = response.problem(409);
            assert_eq!(
                problem.detail.as_deref(),
                Some("the course is archived"),
                "{method} {path}"
            );
        }

        // Someone who may not do it is told so, not that the course is archived.
        assert_eq!(
            w.status_of("PATCH", &w.student, &course, Some(json!({"title": "New"})))
                .await,
            403
        );
        // Everything that only reads still works, grades included.
        w.get(&w.owner, &format!("{course}/gradebook")).await;
        w.get(&w.student, &format!("{course}/my-grades")).await;
        w.get(&w.owner, &format!("/assignments/{assignment}/submissions"))
            .await;
        w.get(&w.owner, &format!("{course}/people")).await;
        // The title did not change.
        assert_eq!(w.get(&w.owner, &course).await["title"], "C");
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// Deletion
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn an_empty_course_is_deleted_with_everything_in_it() {
    db_test(async |pool| {
        let w = world(pool).await;
        let assignment = w.assignment("20", false, false).await;
        // A draft and an empty grade row are not results: they go with the course.
        let base = format!("/assignments/{assignment}/submission/draft");
        w.env
            .ok(
                "PUT",
                &w.student.token,
                &base,
                Some(json!({"content": essay("draft")})),
                200,
            )
            .await;
        w.env
            .ok(
                "PUT",
                &w.owner.token,
                &format!("/assignments/{assignment}/grades/{}", w.other.id),
                Some(json!({"score": null, "feedback": null})),
                200,
            )
            .await;
        let course = format!("/courses/{}", w.course);

        assert_eq!(
            w.status_of("DELETE", &w.assistant, &course, None).await,
            403
        );
        assert_eq!(w.status_of("DELETE", &w.student, &course, None).await, 403);
        assert_eq!(w.status_of("DELETE", &w.stranger, &course, None).await, 404);
        w.get(&w.owner, &course).await;

        w.env.ok("DELETE", &w.owner.token, &course, None, 204).await;

        w.env.ok("GET", &w.owner.token, &course, None, 404).await;
        let left = sqlx::query!(
            r#"SELECT (SELECT count(*) FROM content.modules) AS "modules!",
                      (SELECT count(*) FROM content.items) AS "items!",
                      (SELECT count(*) FROM teaching.memberships) AS "members!",
                      (SELECT count(*) FROM gradebook.grades) AS "grades!",
                      (SELECT count(*) FROM gradebook.grade_history) AS "history!",
                      (SELECT count(*) FROM assessment.submissions) AS "submissions!""#
        )
        .fetch_one(&w.env.pool)
        .await
        .unwrap();
        assert_eq!(
            (
                left.modules,
                left.items,
                left.members,
                left.grades,
                left.history,
                left.submissions
            ),
            (0, 0, 0, 0, 0, 0)
        );
    })
    .await;
}

#[tokio::test]
async fn a_course_with_a_grade_or_a_submitted_attempt_is_not_deleted() {
    db_test(async |pool| {
        let w = world(pool).await;
        let assignment = w.assignment("20", false, false).await;
        let course = format!("/courses/{}", w.course);

        // A submitted attempt, not graded yet.
        w.submit(&w.student, assignment).await;
        let refused = w.env.call("DELETE", &w.owner.token, &course, None).await;
        refused.problem(409);

        // A draft grade of a student who never submitted.
        w.grade(assignment, &w.other, "3").await;
        let refused = w.env.call("DELETE", &w.owner.token, &course, None).await;
        refused.problem(409);

        w.get(&w.owner, &course).await;
        let kept = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM gradebook.grades"#)
            .fetch_one(&w.env.pool)
            .await
            .unwrap();
        assert_eq!(kept, 1, "nothing was purged by the refused delete");
    })
    .await;
}

#[tokio::test]
async fn an_archived_course_is_deleted_by_an_admin_and_audited() {
    db_test(async |pool| {
        let w = world(pool).await;
        w.archive().await;
        let course = format!("/courses/{}", w.course);

        w.env.ok("DELETE", &w.admin.token, &course, None, 204).await;

        w.env.ok("GET", &w.admin.token, &course, None, 404).await;
        assert!(
            w.audit_actions()
                .await
                .contains(&"course.delete".to_owned())
        );
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------------------------

/// A sub-module, a lecture and two assignments with deadlines, in a published course.
async fn furnish(w: &World) {
    let sub = id_of(
        &w.env
            .ok(
                "POST",
                &w.owner.token,
                &format!("/courses/{}/modules", w.course),
                Some(json!({"title": "Sub", "parent_id": w.module})),
                201,
            )
            .await,
    );
    w.env
        .ok(
            "POST",
            &w.owner.token,
            &format!("/modules/{sub}/items"),
            Some(json!({"kind": "text", "title": "Lecture 1"})),
            201,
        )
        .await;
    let body = json!({"kind": "assignment", "title": "Essay",
        "assignment": {"type": "text", "max_points": "40", "max_attempts": 2,
                       "due_at": "2030-01-01T12:00:00Z", "on_deadline": "keep_draft", "accept_late": true}});
    let essay_item = id_of(
        &w.env
            .ok(
                "POST",
                &w.owner.token,
                &format!("/modules/{sub}/items"),
                Some(body),
                201,
            )
            .await,
    );
    w.env
        .ok(
            "POST",
            &w.owner.token,
            &format!("/items/{essay_item}/publish"),
            None,
            200,
        )
        .await;
    w.env
        .ok(
            "POST",
            &w.owner.token,
            &format!("/modules/{}/items", w.module),
            Some(json!({"kind": "assignment", "title": "Bonus", "assignment":
                {"type": "no_submission", "max_points": "5", "is_bonus": true}})),
            201,
        )
        .await;
}

#[tokio::test]
async fn a_template_is_a_draft_copy_of_the_tree_without_dates_people_or_results() {
    db_test(async |pool| {
        let w = world(pool).await;
        furnish(&w).await;
        w.env
            .ok(
                "PATCH",
                &w.owner.token,
                &format!("/courses/{}/info", w.course),
                Some(json!({"code": "CS 305", "description": "Algorithms"})),
                200,
            )
            .await;
        w.env
            .ok(
                "POST",
                &w.owner.token,
                &format!("/modules/{}/publish", w.module),
                Some(json!({"cascade": true})),
                200,
            )
            .await;

        // An archived course can still be copied.
        w.archive().await;
        let template = w
            .env
            .ok("POST", &w.owner.token, &format!("/courses/{}/template", w.course), Some(json!({})), 201)
            .await;

        assert_eq!(template["kind"], "template");
        assert_eq!(template["status"], "draft");
        assert_eq!(template["title"], "C");
        assert_eq!(template["owner_id"], w.owner.id.to_string());
        assert_eq!(template["term_id"], Value::Null);
        let id = id_of(&template);
        let info = w.get(&w.owner, &format!("/courses/{id}/info")).await;
        assert_eq!((info["code"].as_str(), info["description"].as_str()), (Some("CS 305"), Some("Algorithms")));
        assert_eq!(info["student_count"], 0, "no members are copied");

        let rows = sqlx::query!(
            r#"SELECT m.title AS module, m.status AS module_status, m.parent_id IS NOT NULL AS "is_sub!",
                      m.publish_at IS NULL AS "no_publish_at!",
                      i.title AS "item?", i.status AS "item_status?", i.kind AS "kind?",
                      a.type AS "type?", a.max_points AS "max_points?", a.is_bonus AS "is_bonus?",
                      a.max_attempts AS "max_attempts?", a.due_at AS "due_at?", a.on_deadline AS "on_deadline?"
               FROM content.modules m
               LEFT JOIN content.items i ON i.module_id = m.id
               LEFT JOIN content.assignments a ON a.item_id = i.id
               WHERE m.course_id = $1 ORDER BY m.parent_id NULLS FIRST, i.position"#,
            id
        )
        .fetch_all(&w.env.pool)
        .await
        .unwrap();
        assert_eq!(rows.len(), 3, "the bonus in the module, the lecture and the essay in the sub-module");
        assert!(rows.iter().all(|r| r.module_status == "draft" && r.no_publish_at));
        assert!(rows.iter().filter_map(|r| r.item_status.as_deref()).all(|s| s == "draft"));
        let essay_row = rows.iter().find(|r| r.item.as_deref() == Some("Essay")).unwrap();
        assert!(essay_row.is_sub);
        assert_eq!(essay_row.max_points.map(|p| p.to_string()).as_deref(), Some("40.00"));
        assert_eq!(essay_row.max_attempts, Some(2));
        assert!(essay_row.due_at.is_none() && essay_row.on_deadline.is_none(), "deadlines are not copied");
        let bonus = rows.iter().find(|r| r.item.as_deref() == Some("Bonus")).unwrap();
        assert_eq!((bonus.r#type.as_deref(), bonus.is_bonus), (Some("no_submission"), Some(true)));

        // The source is untouched.
        let source = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM content.items WHERE course_id = $1 AND status = 'published'"#,
            w.course
        )
        .fetch_one(&w.env.pool)
        .await
        .unwrap();
        assert!(source >= 1);
    })
    .await;
}

#[tokio::test]
async fn a_course_made_from_a_template_is_a_draft_owned_by_the_caller_with_the_trees_copy() {
    db_test(async |pool| {
        let w = world(pool).await;
        furnish(&w).await;
        let template = w
            .env
            .ok(
                "POST",
                &w.owner.token,
                &format!("/courses/{}/template", w.course),
                Some(json!({"title": "Algorithms template"})),
                201,
            )
            .await;
        let template_id = id_of(&template);
        let term = w
            .env
            .ok(
                "POST",
                &w.admin.token,
                "/terms",
                Some(json!({"name": "Spring 2027", "starts_on": "2027-02-01", "ends_on": "2027-06-30"})),
                201,
            )
            .await;

        let path = format!("/templates/{template_id}/courses");
        let course = w
            .env
            .ok(
                "POST",
                &w.owner.token,
                &path,
                Some(json!({"title": "Algorithms, spring", "term_id": term["id"]})),
                201,
            )
            .await;

        assert_eq!(course["kind"], "regular");
        assert_eq!(course["status"], "draft");
        assert_eq!(course["title"], "Algorithms, spring");
        assert_eq!(course["term_id"], term["id"]);
        let tree = w.get(&w.owner, &format!("/courses/{}/content", id_of(&course))).await;
        let top = &tree["modules"].as_array().unwrap()[0];
        assert_eq!(top["title"], "M");
        assert_eq!(top["status"], "draft");
        assert_eq!(top["submodules"][0]["title"], "Sub");
        assert_eq!(top["submodules"][0]["items"].as_array().unwrap().len(), 2);
        // The title defaults to the template's.
        let named = w.env.ok("POST", &w.owner.token, &path, Some(json!({})), 201).await;
        assert_eq!(named["title"], "Algorithms template");

        // Bad input and bad targets.
        w.env.call("POST", &w.owner.token, &path, Some(json!({"term_id": Uuid::now_v7()}))).await.problem(422);
        w.env.call("POST", &w.owner.token, &path, Some(json!({"title": ""}))).await.problem(422);
        w.env
            .ok("POST", &w.owner.token, &format!("/templates/{}/courses", w.course), Some(json!({})), 404)
            .await;
        assert_eq!(w.status_of("POST", &w.stranger, &path, Some(json!({}))).await, 404);
        assert_eq!(w.status_of("POST", &w.student, &path, Some(json!({}))).await, 404);
    })
    .await;
}

#[tokio::test]
async fn the_templates_of_a_teacher_are_listed_with_what_they_hold() {
    db_test(async |pool| {
        let w = world(pool).await;
        furnish(&w).await;
        for title in ["First", "Second", "Third"] {
            w.env
                .ok(
                    "POST",
                    &w.owner.token,
                    &format!("/courses/{}/template", w.course),
                    Some(json!({"title": title})),
                    201,
                )
                .await;
        }

        let page = w.get(&w.owner, "/me/templates?limit=2").await;

        let items = page["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert!(page["next_cursor"].is_string());
        assert_eq!(items[0]["course"]["title"], "First");
        assert_eq!(
            (
                items[0]["sections"].as_u64(),
                items[0]["lectures"].as_u64(),
                items[0]["assignments"].as_u64()
            ),
            (Some(2), Some(1), Some(2))
        );
        let next = w
            .get(
                &w.owner,
                &format!(
                    "/me/templates?limit=2&cursor={}",
                    page["next_cursor"].as_str().unwrap()
                ),
            )
            .await;
        assert_eq!(next["items"].as_array().unwrap().len(), 1);
        assert_eq!(next["next_cursor"], Value::Null);
        // Others have none, and a regular course is not a template.
        assert_eq!(
            w.get(&w.stranger, "/me/templates").await["items"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    })
    .await;
}

#[tokio::test]
async fn a_student_or_an_outsider_cannot_make_a_template() {
    db_test(async |pool| {
        let w = world(pool).await;
        let path = format!("/courses/{}/template", w.course);
        assert_eq!(
            w.status_of("POST", &w.student, &path, Some(json!({})))
                .await,
            403
        );
        assert_eq!(
            w.status_of("POST", &w.stranger, &path, Some(json!({})))
                .await,
            404
        );
        // An assistant who is a teacher at the institution may.
        assert_eq!(
            w.status_of("POST", &w.assistant, &path, Some(json!({})))
                .await,
            201
        );
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// People and candidates
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn staff_see_emails_former_students_and_groups_and_a_student_sees_only_names() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = id_of(
            &w.env
                .ok(
                    "POST",
                    &w.admin.token,
                    "/groups",
                    Some(json!({"name": "CS-31"})),
                    201,
                )
                .await,
        );
        let grouped = w.env.user("grouped@lms.test", Role::Student).await;
        w.env
            .ok(
                "POST",
                &w.admin.token,
                &format!("/groups/{group}/members"),
                Some(json!({"user_ids": [grouped.id]})),
                200,
            )
            .await;
        w.env
            .ok(
                "POST",
                &w.owner.token,
                &format!("/courses/{}/groups", w.course),
                Some(json!({"group_id": group})),
                201,
            )
            .await;
        // A former student: graded, then off the roster.
        let assignment = w.assignment("20", false, false).await;
        w.submit(&w.other, assignment).await;
        w.env
            .ok(
                "DELETE",
                &w.owner.token,
                &format!("/courses/{}/members/{}", w.course, w.other.id),
                None,
                204,
            )
            .await;
        let path = format!("/courses/{}/people", w.course);

        let staff = w.get(&w.owner, &path).await;
        let people = staff["people"].as_array().unwrap();
        let by_email = |email: &str| people.iter().find(|p| p["email"] == email).unwrap();
        assert_eq!(by_email("owner@lms.test")["is_owner"], true);
        assert_eq!(by_email("owner@lms.test")["role"], "teacher");
        assert_eq!(by_email("assistant@lms.test")["role"], "assistant");
        assert_eq!(by_email("grouped@lms.test")["source"], "group");
        assert_eq!(by_email("grouped@lms.test")["group"], "CS-31");
        assert_eq!(by_email("student@lms.test")["source"], "direct");
        assert_eq!(by_email("student@lms.test")["group"], Value::Null);
        assert_eq!(by_email("other@lms.test")["former"], true);
        // Staff come first.
        assert_eq!(people[0]["role"], "teacher");
        assert_eq!(people[1]["role"], "assistant");
        let groups = staff["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(
            (
                groups[0]["group_name"].as_str(),
                groups[0]["subgroup_name"].is_null(),
                groups[0]["student_count"].as_u64()
            ),
            (Some("CS-31"), true, Some(1))
        );

        let student = w.get(&w.student, &path).await;
        let seen = student["people"].as_array().unwrap();
        assert!(
            seen.iter()
                .all(|p| p["email"].is_null() && p["former"] == false),
            "{seen:?}"
        );
        assert!(!student.to_string().contains("@lms.test"), "{student}");
        assert!(student["groups"].as_array().unwrap().is_empty());
        assert!(
            names(&student["people"]).len() == seen.len() && seen.len() == 4,
            "owner, assistant, two students"
        );

        w.env.ok("GET", &w.stranger.token, &path, None, 404).await;
    })
    .await;
}

#[tokio::test]
async fn a_subgroup_link_shows_its_name_after_the_groups() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = id_of(
            &w.env
                .ok(
                    "POST",
                    &w.admin.token,
                    "/groups",
                    Some(json!({"name": "CS-32"})),
                    201,
                )
                .await,
        );
        let sub = id_of(
            &w.env
                .ok(
                    "POST",
                    &w.admin.token,
                    &format!("/groups/{group}/subgroups"),
                    Some(json!({"name": "A"})),
                    201,
                )
                .await,
        );
        let member = w.env.user("sub@lms.test", Role::Student).await;
        w.env
            .ok(
                "POST",
                &w.admin.token,
                &format!("/groups/{group}/members"),
                Some(json!({"user_ids": [member.id]})),
                200,
            )
            .await;
        w.env
            .ok(
                "POST",
                &w.admin.token,
                &format!("/subgroups/{sub}/members"),
                Some(json!({"user_ids": [member.id]})),
                200,
            )
            .await;
        w.env
            .ok(
                "POST",
                &w.owner.token,
                &format!("/courses/{}/groups", w.course),
                Some(json!({"group_id": group, "subgroup_id": sub})),
                201,
            )
            .await;

        let people = w
            .get(&w.owner, &format!("/courses/{}/people", w.course))
            .await;

        let row = people["people"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["email"] == "sub@lms.test")
            .unwrap();
        assert_eq!(row["group"], "CS-32 / A");
        assert_eq!(people["groups"][0]["subgroup_name"], "A");
    })
    .await;
}

#[tokio::test]
async fn candidates_are_active_people_off_the_course_found_by_name_surname_or_email() {
    db_test(async |pool| {
        let w = world(pool).await;
        let ana = w.env.user("ana.kovalenko@lms.test", Role::Student).await;
        let gone = w.env.user("anton.gone@lms.test", Role::Student).await;
        w.env
            .ok(
                "PATCH",
                &w.admin.token,
                &format!("/admin/users/{}", gone.id),
                Some(json!({"status": "disabled"})),
                200,
            )
            .await;
        w.env
            .ok(
                "PATCH",
                &w.admin.token,
                &format!("/admin/users/{}", ana.id),
                Some(json!({"name": "Ana", "surname": "Kovalenko"})),
                200,
            )
            .await;
        let search = |q: &str| format!("/courses/{}/candidates?q={q}", w.course);

        let by_surname = w.get(&w.owner, &search("KOVAL")).await;
        assert_eq!(by_surname["items"].as_array().unwrap().len(), 1);
        assert_eq!(by_surname["items"][0]["email"], "ana.kovalenko@lms.test");
        assert_eq!(by_surname["items"][0]["role"], "student");
        assert_eq!(
            w.get(&w.owner, &search("ana%20kov")).await["items"]
                .as_array()
                .unwrap()
                .len(),
            1,
            "name and surname together"
        );
        assert!(
            !w.get(&w.owner, &search("lms.test")).await["items"]
                .as_array()
                .unwrap()
                .is_empty(),
            "the address matches too"
        );

        // Not the people on the course, the disabled, the admins or the owner.
        let all: Vec<String> = w.get(&w.owner, &search("lms")).await["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["email"].as_str().unwrap().to_owned())
            .collect();
        for absent in [
            "student@lms.test",
            "other@lms.test",
            "assistant@lms.test",
            "owner@lms.test",
            "admin@lms.test",
            "anton.gone@lms.test",
        ] {
            assert!(!all.contains(&absent.to_owned()), "{absent}: {all:?}");
        }
        assert!(all.contains(&"stranger@lms.test".to_owned()));
        // The assistant manages the roster, so they may search too.
        w.get(&w.assistant, &search("kov")).await;
    })
    .await;
}

#[tokio::test]
async fn a_search_takes_at_least_two_characters_matches_pattern_characters_as_they_are_and_is_capped()
 {
    db_test(async |pool| {
        let w = world(pool).await;
        let search = |q: &str| format!("/courses/{}/candidates?q={q}", w.course);
        w.env
            .call("GET", &w.owner.token, &search("a"), None)
            .await
            .problem(422);
        w.env
            .call("GET", &w.owner.token, &search("%20%20%20"), None)
            .await
            .problem(422);
        w.env
            .call(
                "GET",
                &w.owner.token,
                &format!("/courses/{}/candidates", w.course),
                None,
            )
            .await
            .problem(422);

        // `%` and `_` are plain characters: they match nobody here, instead of everybody.
        for q in ["%25%25", "_%5F", "%5C%5C"] {
            assert!(
                w.get(&w.owner, &search(q)).await["items"]
                    .as_array()
                    .unwrap()
                    .is_empty(),
                "{q}"
            );
        }

        for i in 0..25 {
            w.env
                .user(&format!("bulk{i:02}@lms.test"), Role::Student)
                .await;
        }
        let found = w.get(&w.owner, &search("bulk")).await;
        assert_eq!(found["items"].as_array().unwrap().len(), 20);
        assert_eq!(found["items"][0]["email"], "bulk00@lms.test");

        assert_eq!(
            w.status_of("GET", &w.student, &search("bulk"), None).await,
            403
        );
        assert_eq!(
            w.status_of("GET", &w.stranger, &search("bulk"), None).await,
            404
        );
    })
    .await;
}

#[tokio::test]
async fn every_course_endpoint_of_this_task_is_in_the_openapi_document() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let doc = env.http.get("/openapi.json").send().await.json();
        for (path, method) in [
            ("/api/v1/courses/{id}/info", "get"),
            ("/api/v1/courses/{id}/info", "patch"),
            ("/api/v1/courses/{id}/archive", "post"),
            ("/api/v1/courses/{id}/restore", "post"),
            ("/api/v1/courses/{id}", "delete"),
            ("/api/v1/courses/{id}/template", "post"),
            ("/api/v1/templates/{id}/courses", "post"),
            ("/api/v1/me/templates", "get"),
            ("/api/v1/courses/{id}/people", "get"),
            ("/api/v1/courses/{id}/candidates", "get"),
        ] {
            assert!(doc["paths"][path][method].is_object(), "{method} {path}");
        }
    })
    .await;
}
