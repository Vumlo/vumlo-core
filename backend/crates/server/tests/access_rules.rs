//! ADR-0092: completion and access rules. A module or an item can be locked until the student has
//! finished an item or a module, or reached a score. Locked is not hidden: the title stays in the
//! tree, and the text, the questions and the right to submit wait for the rule.

mod common;

use ::common::Role;
use common::{Account, Env, db_test, essay, id_of};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

struct World {
    env: Env,
    teacher: Account,
    student: Account,
    outsider: Account,
    course: Uuid,
    module: Uuid,
}

async fn world(pool: PgPool) -> World {
    let env = Env::new(pool).await;
    let teacher = env.user("teacher@x.edu", Role::Teacher).await;
    let student = env.user("student@x.edu", Role::Student).await;
    let outsider = env.user("outsider@x.edu", Role::Teacher).await;
    let (course, module) = env.course(&teacher, &[&student]).await;
    World {
        env,
        teacher,
        student,
        outsider,
        course,
        module,
    }
}

/// A condition `completed` on an item.
fn done_item(item: Uuid) -> Value {
    json!({"kind": "completed", "item_id": item})
}

fn done_module(module: Uuid) -> Value {
    json!({"kind": "completed", "module_id": module})
}

fn score(assignment: Uuid, points: &str) -> Value {
    json!({"kind": "score_at_least", "item_id": assignment, "points": points})
}

fn rule(conditions: &[Value]) -> Value {
    json!({"access_rule": {"all": conditions}})
}

impl World {
    async fn ok(
        &self,
        who: &Account,
        method: &str,
        path: &str,
        body: Option<Value>,
        status: u16,
    ) -> Value {
        self.env.ok(method, &who.token, path, body, status).await
    }

    /// A published text item with `text` in its body.
    async fn lecture(&self, module: Uuid, title: &str, text: &str) -> Uuid {
        let item = id_of(
            &self
                .ok(
                    &self.teacher,
                    "POST",
                    &format!("/modules/{module}/items"),
                    Some(json!({"kind": "text", "title": title, "body": essay(text)})),
                    201,
                )
                .await,
        );
        self.publish(item).await;
        item
    }

    /// A published text assignment worth `points`.
    async fn task(&self, title: &str, points: &str) -> Uuid {
        let item = id_of(
            &self
                .ok(
                    &self.teacher,
                    "POST",
                    &format!("/modules/{}/items", self.module),
                    Some(json!({"kind": "assignment", "title": title,
                                "assignment": {"type": "text", "max_points": points}})),
                    201,
                )
                .await,
        );
        self.publish(item).await;
        item
    }

    async fn publish(&self, item: Uuid) {
        self.ok(
            &self.teacher,
            "POST",
            &format!("/items/{item}/publish"),
            None,
            200,
        )
        .await;
    }

    /// A published module, or a published sub-module of `parent`.
    async fn module_in(&self, title: &str, parent: Option<Uuid>) -> Uuid {
        let module = id_of(
            &self
                .ok(
                    &self.teacher,
                    "POST",
                    &format!("/courses/{}/modules", self.course),
                    Some(json!({"title": title, "parent_id": parent})),
                    201,
                )
                .await,
        );
        self.ok(
            &self.teacher,
            "POST",
            &format!("/modules/{module}/publish"),
            Some(json!({"cascade": true})),
            200,
        )
        .await;
        module
    }

    /// Changes an item (`item`) or a module (`module`) as the teacher and returns the response.
    async fn set(&self, kind: &str, id: Uuid, body: Value, status: u16) -> Value {
        self.set_as(&self.teacher, kind, id, body, status).await
    }

    async fn set_as(&self, who: &Account, kind: &str, id: Uuid, body: Value, status: u16) -> Value {
        self.ok(who, "PATCH", &format!("/{kind}s/{id}"), Some(body), status)
            .await
    }

    async fn require(&self, item: Uuid, conditions: &[Value]) {
        self.set("item", item, rule(conditions), 200).await;
    }

    async fn tree(&self, who: &Account) -> Value {
        self.ok(
            who,
            "GET",
            &format!("/courses/{}/content", self.course),
            None,
            200,
        )
        .await
    }

    async fn item(&self, who: &Account, id: Uuid) -> Value {
        self.ok(who, "GET", &format!("/items/{id}"), None, 200)
            .await
    }

    /// What the student is told about an item in the tree: its `standing`.
    async fn standing(&self, id: Uuid) -> Value {
        node(&self.tree(&self.student).await, id)["standing"].clone()
    }

    async fn locked(&self, id: Uuid) -> bool {
        !self.standing(id).await["locked"]
            .as_array()
            .unwrap()
            .is_empty()
    }

    async fn is_done(&self, id: Uuid) -> bool {
        self.standing(id).await["done"] == json!(true)
    }

    /// The hits the student's search for `q` finds.
    async fn search(&self, q: &str) -> Vec<Value> {
        let found = self
            .ok(&self.student, "GET", &format!("/search?q={q}"), None, 200)
            .await;
        found["items"].as_array().unwrap().clone()
    }

    async fn mark(&self, id: Uuid) {
        self.ok(
            &self.student,
            "PUT",
            &format!("/items/{id}/completion"),
            None,
            204,
        )
        .await;
    }

    /// The student submits a text answer to the assignment.
    async fn submit(&self, assignment: Uuid) {
        self.ok(
            &self.student,
            "PUT",
            &format!("/assignments/{assignment}/submission/draft"),
            Some(json!({"content": essay("answer")})),
            200,
        )
        .await;
        self.ok(
            &self.student,
            "POST",
            &format!("/assignments/{assignment}/submission/submit"),
            None,
            201,
        )
        .await;
    }

    /// Grades the student's work and publishes the grade.
    async fn grade(&self, assignment: Uuid, points: &str) {
        self.ok(
            &self.teacher,
            "PUT",
            &format!("/assignments/{assignment}/grades/{}", self.student.id),
            Some(json!({"score": points, "feedback": null})),
            200,
        )
        .await;
        self.ok(
            &self.teacher,
            "POST",
            &format!("/assignments/{assignment}/grades/publish"),
            Some(json!({"students": "all"})),
            200,
        )
        .await;
    }
}

/// The module or the item with `id` anywhere in a course tree.
fn node(tree: &Value, id: Uuid) -> Value {
    fn find(value: &Value, id: &str) -> Option<Value> {
        match value {
            Value::Object(map) => {
                if map.get("id").and_then(Value::as_str) == Some(id) {
                    return Some(value.clone());
                }
                map.values().find_map(|inner| find(inner, id))
            }
            Value::Array(list) => list.iter().find_map(|inner| find(inner, id)),
            _ => None,
        }
    }
    find(tree, &id.to_string()).unwrap_or_else(|| panic!("{id} is not in the tree"))
}

// --- setting a rule ----------------------------------------------------------------------------

#[tokio::test]
async fn a_teacher_sets_reads_and_removes_a_rule_and_a_student_never_reads_it() {
    db_test(async |pool| {
        let w = world(pool).await;
        let first = w.lecture(w.module, "First", "one").await;
        let second = w.lecture(w.module, "Second", "two").await;

        let updated = w.set("item", second, rule(&[done_item(first)]), 200).await;
        assert_eq!(updated["access_rule"], json!({"all": [done_item(first)]}));
        // The rule is on the item, and in the tree, for staff.
        assert_eq!(
            w.item(&w.teacher, second).await["access_rule"],
            updated["access_rule"]
        );
        assert_eq!(
            node(&w.tree(&w.teacher).await, second)["access_rule"],
            updated["access_rule"]
        );
        // Staff are never locked and are told nothing about where they stand.
        assert!(w.item(&w.teacher, second).await.get("standing").is_none());
        assert_eq!(w.item(&w.teacher, second).await["body"], essay("two"));
        // A student reads where they stand, never the rule.
        assert!(
            w.item(&w.student, second)
                .await
                .get("access_rule")
                .is_none()
        );
        assert!(
            node(&w.tree(&w.student).await, second)
                .get("access_rule")
                .is_none()
        );

        // `null` removes it, and so does an empty rule.
        let removed = w
            .set("item", second, json!({"access_rule": null}), 200)
            .await;
        assert!(removed.get("access_rule").is_none());
        w.require(second, &[done_item(first)]).await;
        let emptied = w.set("item", second, rule(&[]), 200).await;
        assert!(emptied.get("access_rule").is_none());
        // Leaving the field out leaves the rule alone.
        w.require(second, &[done_item(first)]).await;
        let renamed = w
            .set("item", second, json!({"title": "Second, renamed"}), 200)
            .await;
        assert_eq!(renamed["access_rule"], json!({"all": [done_item(first)]}));
    })
    .await;
}

#[tokio::test]
async fn a_rule_must_name_something_in_this_course_and_not_close_a_circle() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = w.lecture(w.module, "A", "a").await;
        let b = w.lecture(w.module, "B", "b").await;
        let task = w.task("Task", "10").await;

        let refused = |body: Value| async { w.set("item", b, body, 422).await };
        // Something of another course, something that does not exist, itself, both kinds at once.
        let (_, other_module) = w.env.course(&w.outsider, &[]).await;
        let foreign = id_of(
            &w.env
                .ok(
                    "POST",
                    &w.outsider.token,
                    &format!("/modules/{other_module}/items"),
                    Some(json!({"kind": "text", "title": "Elsewhere"})),
                    201,
                )
                .await,
        );
        for (condition, field) in [
            (done_item(foreign), "access_rule.all[0].item_id"),
            (done_item(Uuid::now_v7()), "access_rule.all[0].item_id"),
            (done_module(Uuid::now_v7()), "access_rule.all[0].module_id"),
            (done_item(b), "access_rule.all[0].item_id"),
            (
                json!({"kind": "completed", "item_id": a, "module_id": w.module}),
                "access_rule.all[0].item_id",
            ),
            (
                json!({"kind": "completed", "item_id": a, "points": "1"}),
                "access_rule.all[0].points",
            ),
            // A score is asked of an assignment, for points the assignment can give.
            (score(a, "1"), "access_rule.all[0].item_id"),
            (score(task, "11"), "access_rule.all[0].points"),
            (score(task, "0"), "access_rule.all[0].points"),
            (score(task, "2.555"), "access_rule.all[0].points"),
            (
                json!({"kind": "score_at_least", "item_id": task}),
                "access_rule.all[0].item_id",
            ),
        ] {
            let problem = refused(rule(std::slice::from_ref(&condition))).await;
            assert_eq!(problem["errors"][0]["field"], field, "{condition}");
        }
        let many = vec![done_item(a); 11];
        assert_eq!(
            refused(rule(&many)).await["errors"][0]["field"],
            "access_rule.all"
        );

        // B waits for A; A waiting for B would close a circle, and so would A waiting for A's module.
        w.require(b, &[done_item(a)]).await;
        let circle = w.set("item", a, rule(&[done_item(b)]), 422).await;
        assert_eq!(circle["errors"][0]["field"], "access_rule");
        let inside = w.set("item", a, rule(&[done_module(w.module)]), 422).await;
        assert_eq!(inside["errors"][0]["field"], "access_rule");
        // A refused rule changed nothing.
        assert!(w.item(&w.teacher, a).await.get("access_rule").is_none());
    })
    .await;
}

#[tokio::test]
async fn only_staff_of_the_course_set_a_rule() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = w.lecture(w.module, "A", "a").await;
        let b = w.lecture(w.module, "B", "b").await;
        let body = rule(&[done_item(a)]);
        w.set_as(&w.student, "item", b, body.clone(), 403).await;
        w.set_as(&w.outsider, "item", b, body, 404).await;
        assert!(w.item(&w.teacher, b).await.get("access_rule").is_none());
    })
    .await;
}

// --- what a student sees -----------------------------------------------------------------------

#[tokio::test]
async fn a_locked_item_is_its_title_until_the_assignment_it_waits_for_is_submitted() {
    db_test(async |pool| {
        let w = world(pool).await;
        let task = w.task("Homework", "10").await;
        let next = w.lecture(w.module, "Next lecture", "the secret text").await;
        w.require(next, &[done_item(task)]).await;

        // Visible but closed: the tree names what is missing, the item has no text.
        assert!(w.locked(next).await);
        let tree = w.standing(next).await;
        assert_eq!(tree["locked"][0]["title"], "Homework");
        assert_eq!(tree["locked"][0]["condition"], done_item(task));
        let item = w.item(&w.student, next).await;
        assert_eq!(item["title"], "Next lecture");
        assert_eq!(item["body"], json!({}));
        assert_eq!(item["standing"]["locked"][0]["title"], "Homework");

        w.submit(task).await;
        assert!(w.is_done(task).await);
        assert!(!w.locked(next).await);
        assert_eq!(
            w.item(&w.student, next).await["body"],
            essay("the secret text")
        );
    })
    .await;
}

#[tokio::test]
async fn a_text_item_is_done_when_the_student_marks_it_and_not_before() {
    db_test(async |pool| {
        let w = world(pool).await;
        let read = w.lecture(w.module, "Read this", "text").await;
        let after = w.lecture(w.module, "After reading", "more").await;
        w.require(after, &[done_item(read)]).await;

        assert!(!w.is_done(read).await);
        assert!(w.locked(after).await);
        w.mark(read).await;
        w.mark(read).await; // marking again changes nothing
        assert!(w.is_done(read).await);
        assert!(!w.locked(after).await);

        w.ok(
            &w.student,
            "DELETE",
            &format!("/items/{read}/completion"),
            None,
            204,
        )
        .await;
        w.ok(
            &w.student,
            "DELETE",
            &format!("/items/{read}/completion"),
            None,
            204,
        )
        .await;
        assert!(!w.is_done(read).await);
        assert!(w.locked(after).await);
    })
    .await;
}

#[tokio::test]
async fn an_assignment_takes_no_mark_and_staff_and_locked_items_are_refused() {
    db_test(async |pool| {
        let w = world(pool).await;
        let task = w.task("Homework", "10").await;
        let first = w.lecture(w.module, "First", "a").await;
        let second = w.lecture(w.module, "Second", "b").await;
        w.require(second, &[done_item(first)]).await;

        let path = |id: Uuid| format!("/items/{id}/completion");
        // An assignment is done by being submitted or graded.
        w.ok(&w.student, "PUT", &path(task), None, 409).await;
        // An item that is locked has not been read.
        w.ok(&w.student, "PUT", &path(second), None, 409).await;
        // Only a student of the course marks.
        w.ok(&w.teacher, "PUT", &path(first), None, 403).await;
        w.ok(&w.outsider, "PUT", &path(first), None, 404).await;
        // A draft item does not exist for a student.
        let draft = id_of(
            &w.ok(
                &w.teacher,
                "POST",
                &format!("/modules/{}/items", w.module),
                Some(json!({"kind": "text", "title": "Draft"})),
                201,
            )
            .await,
        );
        w.ok(&w.student, "PUT", &path(draft), None, 404).await;
        // An archived course is read-only.
        w.ok(
            &w.teacher,
            "POST",
            &format!("/courses/{}/archive", w.course),
            None,
            200,
        )
        .await;
        w.ok(&w.student, "PUT", &path(first), None, 409).await;
    })
    .await;
}

#[tokio::test]
async fn a_module_is_done_when_every_item_in_it_and_in_its_sub_modules_is() {
    db_test(async |pool| {
        let w = world(pool).await;
        let unit = w.module_in("Unit", None).await;
        let sub = w.module_in("Part", Some(unit)).await;
        let x = w.lecture(unit, "X", "x").await;
        let y = w.lecture(sub, "Y", "y").await;
        let exam = w.lecture(w.module, "Exam", "questions").await;
        w.require(exam, &[done_module(unit)]).await;

        assert!(w.locked(exam).await);
        assert_eq!(w.standing(exam).await["locked"][0]["title"], "Unit");
        w.mark(x).await;
        assert!(!w.is_done(unit).await);
        assert!(w.locked(exam).await);
        w.mark(y).await;
        assert!(w.is_done(unit).await);
        assert!(w.is_done(sub).await);
        assert!(!w.locked(exam).await);
    })
    .await;
}

#[tokio::test]
async fn a_score_condition_waits_for_the_published_score() {
    db_test(async |pool| {
        let w = world(pool).await;
        let test = w.task("Test", "10").await;
        let next = w.lecture(w.module, "Next", "text").await;
        w.require(next, &[score(test, "6")]).await;
        w.submit(test).await;

        // Submitted is not passed: there is no score yet, then not enough, then enough. A draft
        // of the grade is not a score either.
        assert!(w.locked(next).await);
        w.ok(
            &w.teacher,
            "PUT",
            &format!("/assignments/{test}/grades/{}", w.student.id),
            Some(json!({"score": "9", "feedback": null})),
            200,
        )
        .await;
        assert!(w.locked(next).await);
        w.grade(test, "5").await;
        assert!(w.locked(next).await);
        w.grade(test, "6").await;
        assert!(!w.locked(next).await);
    })
    .await;
}

#[tokio::test]
async fn a_rule_on_a_module_locks_what_is_in_it() {
    db_test(async |pool| {
        let w = world(pool).await;
        let gate = w.lecture(w.module, "Gate", "g").await;
        let unit = w.module_in("Unit", None).await;
        let sub = w.module_in("Part", Some(unit)).await;
        let inner = w.lecture(unit, "Inner", "i").await;
        let deep = w.lecture(sub, "Deep", "d").await;
        w.set("module", unit, rule(&[done_item(gate)]), 200).await;

        for node in [unit, sub, inner, deep] {
            assert!(w.locked(node).await, "{node}");
        }
        assert_eq!(w.item(&w.student, deep).await["body"], json!({}));
        w.mark(gate).await;
        for node in [unit, sub, inner, deep] {
            assert!(!w.locked(node).await, "{node}");
        }
        // Staff read the rule on the module.
        let module = node(&w.tree(&w.teacher).await, unit);
        assert_eq!(module["access_rule"], json!({"all": [done_item(gate)]}));
    })
    .await;
}

#[tokio::test]
async fn a_condition_names_only_what_the_student_can_see() {
    db_test(async |pool| {
        let w = world(pool).await;
        let hidden = id_of(
            &w.ok(
                &w.teacher,
                "POST",
                &format!("/modules/{}/items", w.module),
                Some(json!({"kind": "text", "title": "Unpublished plan"})),
                201,
            )
            .await,
        );
        let item = w.lecture(w.module, "Item", "text").await;
        w.require(item, &[done_item(hidden)]).await;
        // The draft's title is not given away.
        let locked = w.standing(item).await["locked"].clone();
        assert_eq!(locked[0]["title"], Value::Null);
        assert_eq!(locked[0]["condition"], done_item(hidden));
    })
    .await;
}

// --- what a locked item stops -------------------------------------------------------------------

#[tokio::test]
async fn a_locked_assignment_takes_no_draft_and_no_submit_until_it_opens() {
    db_test(async |pool| {
        let w = world(pool).await;
        let read = w.lecture(w.module, "Read first", "text").await;
        let task = w.task("Homework", "10").await;
        w.require(task, &[done_item(read)]).await;
        let draft = format!("/assignments/{task}/submission/draft");
        let submit = format!("/assignments/{task}/submission/submit");

        // Its questions are not shown either.
        assert_eq!(
            w.item(&w.student, task).await["assignment"]["config"],
            json!({})
        );
        w.ok(
            &w.student,
            "PUT",
            &draft,
            Some(json!({"content": essay("early")})),
            409,
        )
        .await;
        w.ok(&w.student, "POST", &submit, None, 409).await;

        w.mark(read).await;
        w.ok(
            &w.student,
            "PUT",
            &draft,
            Some(json!({"content": essay("on time")})),
            200,
        )
        .await;
        w.ok(&w.student, "POST", &submit, None, 201).await;
    })
    .await;
}

#[tokio::test]
async fn a_locked_item_is_not_found_by_what_is_in_it() {
    db_test(async |pool| {
        let w = world(pool).await;
        let read = w.lecture(w.module, "Read first", "text").await;
        let later = w
            .lecture(w.module, "Later", "a document about zeppelins")
            .await;
        w.require(later, &[done_item(read)]).await;

        assert!(w.search("zeppelins").await.is_empty());
        // The title is still there to be found, without a quote of the text.
        let by_title = w.search("Later").await;
        assert_eq!(by_title[0]["snippet"], Value::Null);
        w.mark(read).await;
        assert_eq!(w.search("zeppelins").await.len(), 1);
    })
    .await;
}

// --- what keeps a rule true --------------------------------------------------------------------

#[tokio::test]
async fn deleting_what_a_rule_names_removes_the_condition_and_unlocks_the_item() {
    db_test(async |pool| {
        let w = world(pool).await;
        let gone = w.lecture(w.module, "Gone soon", "x").await;
        let kept = w.lecture(w.module, "Kept", "y").await;
        let item = w.lecture(w.module, "Item", "z").await;
        w.require(item, &[done_item(gone), done_item(kept)]).await;
        assert_eq!(
            w.standing(item).await["locked"].as_array().unwrap().len(),
            2
        );

        w.ok(&w.teacher, "DELETE", &format!("/items/{gone}"), None, 204)
            .await;
        assert_eq!(
            w.item(&w.teacher, item).await["access_rule"],
            json!({"all": [done_item(kept)]})
        );
        assert_eq!(
            w.standing(item).await["locked"].as_array().unwrap().len(),
            1
        );
    })
    .await;
}

#[tokio::test]
async fn a_template_and_the_course_made_from_it_keep_the_rules_pointing_at_their_own_copies() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = w.lecture(w.module, "Reading", "a").await;
        let b = w.lecture(w.module, "After reading", "b").await;
        w.require(b, &[done_item(a)]).await;

        let template = id_of(
            &w.ok(
                &w.teacher,
                "POST",
                &format!("/courses/{}/template", w.course),
                Some(json!({})),
                201,
            )
            .await,
        );
        let copy = w
            .ok(
                &w.teacher,
                "GET",
                &format!("/courses/{template}/content"),
                None,
                200,
            )
            .await;
        let find = |title: &str| {
            let mut found = None;
            for module in copy["modules"].as_array().unwrap() {
                for item in module["items"].as_array().unwrap() {
                    if item["title"] == title {
                        found = Some(item.clone());
                    }
                }
            }
            found.unwrap_or_else(|| panic!("{title} was not copied"))
        };
        let (reading, after) = (find("Reading"), find("After reading"));
        let target = after["access_rule"]["all"][0]["item_id"].clone();
        assert_eq!(
            target, reading["id"],
            "the rule names the copy, not the original"
        );
        assert_ne!(target, json!(a));
    })
    .await;
}

#[tokio::test]
async fn a_student_who_finished_before_a_rule_was_set_is_not_locked_by_it() {
    db_test(async |pool| {
        let w = world(pool).await;
        let task = w.task("Homework", "10").await;
        let item = w.lecture(w.module, "Item", "text").await;
        w.submit(task).await;
        w.require(item, &[done_item(task)]).await;
        assert!(!w.locked(item).await);
    })
    .await;
}
