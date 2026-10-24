//! ADR-0052: the course cards, the work list, the teaching overview and search.

mod common;

use ::common::Role;
use common::{Account, Env, db_test, essay, id_of};
use serde_json::{Value, json};
use uuid::Uuid;

struct World {
    env: Env,
    owner: Account,
    student: Account,
    other: Account,
    outsider: Account,
    outsider_student: Account,
    course: Uuid,
    module: Uuid,
}

async fn world(pool: sqlx::PgPool) -> World {
    let env = Env::new(pool).await;
    let owner = env.user("owner@lms.test", Role::Teacher).await;
    let student = env.user("student@lms.test", Role::Student).await;
    let other = env.user("other@lms.test", Role::Student).await;
    let outsider = env.user("outsider@lms.test", Role::Teacher).await;
    let outsider_student = env.user("outsider-student@lms.test", Role::Student).await;
    let (course, module) = env.course(&owner, &[&student, &other]).await;
    World {
        env,
        owner,
        student,
        other,
        outsider,
        outsider_student,
        course,
        module,
    }
}

fn doc(text: &str) -> Value {
    json!({"v": 1, "attachments": [], "blocks": [
        {"id": "p1", "type": "paragraph", "content": [{"type": "text", "text": text}]}]})
}

impl World {
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

    async fn get(&self, who: &Account, path: &str) -> Value {
        self.ok("GET", who, path, None, 200).await
    }

    async fn item(&self, kind: Value, title: &str, body: Option<Value>, publish: bool) -> Uuid {
        let mut request = json!({"kind": "text", "title": title});
        if let Some(body) = body {
            request["body"] = body;
        }
        if kind != Value::Null {
            request = json!({"kind": "assignment", "title": title, "assignment": kind});
        }
        let id = id_of(
            &self
                .ok(
                    "POST",
                    &self.owner,
                    &format!("/modules/{}/items", self.module),
                    Some(request),
                    201,
                )
                .await,
        );
        if publish {
            self.ok(
                "POST",
                &self.owner,
                &format!("/items/{id}/publish"),
                None,
                200,
            )
            .await;
        }
        id
    }

    async fn task(&self, title: &str, points: &str, due: Option<&str>, publish: bool) -> Uuid {
        let mut assignment = json!({"type": "text", "max_points": points});
        if let Some(due) = due {
            assignment["due_at"] = json!(due);
            assignment["on_deadline"] = json!("keep_draft");
            assignment["accept_late"] = json!(false);
        }
        self.item(assignment, title, None, publish).await
    }

    async fn submit(&self, who: &Account, task: Uuid) {
        self.ok(
            "PUT",
            who,
            &format!("/assignments/{task}/submission/draft"),
            Some(json!({"content": essay("my work")})),
            200,
        )
        .await;
        self.ok(
            "POST",
            who,
            &format!("/assignments/{task}/submission/submit"),
            None,
            201,
        )
        .await;
    }

    async fn grade(&self, task: Uuid, student: &Account, score: &str) {
        self.ok(
            "PUT",
            &self.owner,
            &format!("/assignments/{task}/grades/{}", student.id),
            Some(json!({"score": score, "feedback": "DRAFT-NOTE"})),
            200,
        )
        .await;
    }

    async fn publish_grades(&self, task: Uuid) {
        self.ok(
            "POST",
            &self.owner,
            &format!("/assignments/{task}/grades/publish"),
            Some(json!({"students": "all"})),
            200,
        )
        .await;
    }

    async fn cards(&self, who: &Account) -> Vec<Value> {
        self.get(who, "/me/course-cards").await["items"]
            .as_array()
            .unwrap()
            .clone()
    }
}

const FAR: &str = "2099-01-01T10:00:00Z";
const PAST: &str = "2020-01-01T10:00:00Z";

#[tokio::test]
async fn a_student_card_has_their_progress_and_next_deadline_and_nothing_of_the_teachers() {
    db_test(async |pool| {
        let w = world(pool).await;
        let done = w.task("Done", "10", Some(FAR), true).await;
        let graded = w.task("Graded", "10", None, true).await;
        w.task("Later", "10", Some(FAR), true).await;
        w.task("Hidden draft", "10", Some("2098-01-01T10:00:00Z"), false)
            .await;
        w.submit(&w.student, done).await;
        w.submit(&w.student, graded).await;
        w.grade(graded, &w.student, "7").await;
        w.publish_grades(graded).await;

        let cards = w.cards(&w.student).await;
        assert_eq!(cards.len(), 1);
        let card = &cards[0];
        assert_eq!(card["my_role"], "student");
        assert_eq!(
            card["progress"],
            json!({"submitted": 2, "total": 3, "graded": 1})
        );
        assert_eq!(
            card["next"]["title"], "Later",
            "a draft is not the next deadline"
        );
        assert_eq!(card["todo"], json!([]));
        assert_eq!(card["student_count"], 2);
        let raw = card.to_string();
        assert!(
            !raw.contains("DRAFT-NOTE") && !raw.contains("Hidden draft"),
            "{raw}"
        );

        // The other student has handed in nothing.
        let theirs = &w.cards(&w.other).await[0];
        assert_eq!(
            theirs["progress"],
            json!({"submitted": 0, "total": 3, "graded": 0})
        );
        assert_eq!(
            theirs["next"]["title"], "Done",
            "the earliest of the two on the same day is the first made"
        );
    })
    .await;
}

#[tokio::test]
async fn a_teacher_card_has_counted_todo_entries_and_no_sentences() {
    db_test(async |pool| {
        let w = world(pool).await;
        let task = w.task("Essay", "10", Some(FAR), true).await;
        let quiz_free = w.task("Second", "10", None, true).await;
        w.item(Value::Null, "Unwritten lecture", None, false).await;
        w.submit(&w.student, task).await;
        w.submit(&w.other, task).await;
        w.submit(&w.student, quiz_free).await;
        w.grade(quiz_free, &w.student, "5").await;

        let cards = w.cards(&w.owner).await;
        let card = &cards[0];
        assert_eq!(card["my_role"], "owner");
        assert!(card["progress"].is_null() && card["next"].is_null());
        let todo = card["todo"].as_array().unwrap();
        let kinds: Vec<&str> = todo.iter().map(|t| t["kind"].as_str().unwrap()).collect();
        assert_eq!(
            kinds,
            [
                "to_grade",
                "draft_grades",
                "unpublished_items",
                "next_deadline"
            ]
        );
        assert_eq!(todo[0]["count"], 2, "two essays have no score");
        assert_eq!(
            todo[1],
            json!({"kind": "draft_grades", "count": 1, "auto_scored": false})
        );
        assert_eq!(todo[2]["count"], 1);
        assert_eq!(todo[2]["titles"], json!(["Unwritten lecture"]));
        assert_eq!(todo[3]["title"], "Essay");

        w.publish_grades(quiz_free).await;
        let after = &w.cards(&w.owner).await[0];
        let kinds: Vec<&str> = after["todo"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["kind"].as_str().unwrap())
            .collect();
        assert_eq!(
            kinds,
            ["to_grade", "unpublished_items", "next_deadline"],
            "nothing left to publish"
        );
    })
    .await;
}

#[tokio::test]
async fn cards_only_show_the_callers_regular_courses_and_hide_a_draft_course_from_students() {
    db_test(async |pool| {
        let w = world(pool).await;
        let draft = id_of(
            &w.ok(
                "POST",
                &w.owner,
                "/courses",
                Some(json!({"title": "Not yet"})),
                201,
            )
            .await,
        );
        w.ok(
            "PUT",
            &w.owner,
            &format!("/courses/{draft}/members/{}", w.student.id),
            Some(json!({"role": "student"})),
            200,
        )
        .await;
        w.ok(
            "POST",
            &w.owner,
            &format!("/courses/{}/template", w.course),
            Some(json!({})),
            201,
        )
        .await;

        let titles = |cards: &[Value]| -> Vec<String> {
            cards
                .iter()
                .map(|c| c["course"]["title"].as_str().unwrap().to_owned())
                .collect()
        };
        assert_eq!(titles(&w.cards(&w.student).await), ["C"], "no draft course");
        assert_eq!(
            titles(&w.cards(&w.owner).await),
            ["C", "Not yet"],
            "and no template for the owner"
        );
        assert!(w.cards(&w.outsider).await.is_empty());
        assert!(w.cards(&w.outsider_student).await.is_empty());
        assert_eq!(w.env.http.get("/me/course-cards").send().await.status, 401);
    })
    .await;
}

#[tokio::test]
async fn a_course_card_says_whether_its_term_ended_and_names_the_owner() {
    db_test(async |pool| {
        let w = world(pool).await;
        let term = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO org.terms (id, name, starts_on, ends_on) VALUES ($1, 'Old term', '2000-01-01', '2000-06-30')",
            term
        )
        .execute(&w.env.pool)
        .await
        .unwrap();
        sqlx::query!("UPDATE teaching.courses SET term_id = $2, code = 'CS 1' WHERE id = $1", w.course, term)
            .execute(&w.env.pool)
            .await
            .unwrap();
        let card = &w.cards(&w.student).await[0];
        assert_eq!(card["term_name"], "Old term");
        assert_eq!(card["term_ended"], true);
        assert_eq!(card["code"], "CS 1");
        assert!(card["owner_name"].is_string() && card["owner_surname"].is_string());
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// /me/work
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_work_list_has_the_statuses_and_only_published_values() {
    db_test(async |pool| {
        let w = world(pool).await;
        let graded = w.task("Graded", "10", Some(FAR), true).await;
        let sent = w.task("Sent", "10", Some(FAR), true).await;
        let saved = w.task("Saved", "10", Some(FAR), true).await;
        let missed = w.task("Missed", "10", Some(PAST), true).await;
        let fresh = w.task("Fresh", "10", None, true).await;
        let oral = w
            .item(
                json!({"type": "no_submission", "max_points": "5"}),
                "Oral",
                None,
                true,
            )
            .await;
        w.task("Draft task", "10", None, false).await;
        w.submit(&w.student, graded).await;
        w.grade(graded, &w.student, "8").await;
        w.publish_grades(graded).await;
        w.grade(graded, &w.student, "9").await; // a later draft change must not show
        w.submit(&w.student, sent).await;
        w.ok(
            "PUT",
            &w.student,
            &format!("/assignments/{saved}/submission/draft"),
            Some(json!({"content": essay("half")})),
            200,
        )
        .await;

        let work = w.get(&w.student, "/me/work").await;
        assert_eq!(work["courses"].as_array().unwrap().len(), 1);
        assert_eq!(work["courses"][0]["id"], json!(w.course));
        let items = work["items"].as_array().unwrap();
        let of = |id: Uuid| {
            items
                .iter()
                .find(|i| i["assignment_id"] == json!(id))
                .cloned()
                .unwrap_or(Value::Null)
        };
        assert_eq!(of(graded)["status"], "graded");
        assert_eq!(of(graded)["score"], "8.00");
        assert_eq!(of(sent)["status"], "submitted");
        assert_eq!(of(saved)["status"], "draft");
        assert!(of(saved)["draft_saved_at"].is_string());
        assert_eq!(of(missed)["status"], "missed");
        assert_eq!(of(fresh)["status"], "not_started");
        assert_eq!(of(oral)["status"], "in_class");
        assert_eq!(items.len(), 6, "the draft task is not on the list");
        let raw = work.to_string();
        assert!(
            !raw.contains("DRAFT-NOTE") && !raw.contains("Draft task"),
            "{raw}"
        );

        // Another student's work is their own; a teacher studies nowhere.
        let theirs = w.get(&w.other, "/me/work").await;
        assert!(
            theirs["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|i| i["status"] != "graded")
        );
        let teacher = w.get(&w.owner, "/me/work").await;
        assert_eq!(teacher, json!({"courses": [], "items": []}));
    })
    .await;
}

#[tokio::test]
async fn the_work_list_shows_the_published_title_and_the_class_day() {
    db_test(async |pool| {
        let w = world(pool).await;
        let oral = w
            .item(
                json!({"type": "no_submission", "max_points": "5", "class_on": "2099-03-04"}),
                "Oral exam",
                None,
                true,
            )
            .await;
        w.ok(
            "PATCH",
            &w.owner,
            &format!("/items/{oral}"),
            Some(json!({"title": "Oral exam (new room)"})),
            200,
        )
        .await;
        let work = w.get(&w.student, "/me/work").await;
        assert_eq!(work["items"][0]["title"], "Oral exam");
        assert_eq!(work["items"][0]["class_on"], "2099-03-04");
        assert_eq!(work["items"][0]["due_at"], Value::Null);
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// /me/teaching
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_teaching_overview_lists_the_latest_attempts_waiting_for_a_grade() {
    db_test(async |pool| {
        let w = world(pool).await;
        let task = w.task("Essay", "10", Some(FAR), true).await;
        let other_task = w.task("Second", "10", None, true).await;
        w.submit(&w.student, task).await;
        w.submit(&w.student, task).await;
        w.submit(&w.other, task).await;
        w.submit(&w.other, other_task).await;
        w.grade(other_task, &w.other, "4").await;

        let view = w.get(&w.owner, "/me/teaching").await;
        let waiting = view["to_grade"].as_array().unwrap();
        assert_eq!(
            waiting.len(),
            2,
            "one per student, the graded one is gone: {view}"
        );
        let mine = waiting
            .iter()
            .find(|r| r["student_id"] == json!(w.student.id))
            .unwrap();
        assert_eq!(mine["attempt"], 2, "the latest attempt");
        assert_eq!(mine["assignment_title"], "Essay");
        assert_eq!(mine["course_id"], json!(w.course));
        assert_eq!(mine["course_title"], "C");
        assert!(mine["course_code"].is_string());
        assert_eq!(mine["max_attempts"], Value::Null);

        let unpublished = view["unpublished"].as_array().unwrap();
        assert_eq!(unpublished.len(), 1);
        assert_eq!(unpublished[0]["assignment_title"], "Second");
        assert_eq!(unpublished[0]["draft_count"], 1);
        assert_eq!(unpublished[0]["auto_scored"], false);

        let deadlines = view["deadlines"].as_array().unwrap();
        assert_eq!(deadlines.len(), 1);
        assert_eq!(deadlines[0]["title"], "Essay");
        assert_eq!(deadlines[0]["submitted"], 2);
        assert_eq!(deadlines[0]["total"], 2);
        assert_eq!(deadlines[0]["not_open"], false);
    })
    .await;
}

#[tokio::test]
async fn the_teaching_overview_lists_content_students_cannot_read_yet() {
    db_test(async |pool| {
        let w = world(pool).await;
        let lecture = w
            .item(Value::Null, "Published lecture", Some(doc("v1")), true)
            .await;
        w.item(Value::Null, "Draft lecture", None, false).await;
        w.task("Not open", "10", Some(FAR), false).await;
        w.ok(
            "PATCH",
            &w.owner,
            &format!("/items/{lecture}"),
            Some(json!({"body": doc("v2")})),
            200,
        )
        .await;

        let view = w.get(&w.owner, "/me/teaching").await;
        let content = view["unpublished_content"].as_array().unwrap();
        let reason = |title: &str| {
            content
                .iter()
                .find(|c| c["title"] == title)
                .map(|c| c["reason"].clone())
        };
        assert_eq!(reason("Published lecture"), Some(json!("edits")));
        assert_eq!(reason("Draft lecture"), Some(json!("draft")));
        assert_eq!(reason("Not open"), Some(json!("draft")));
        let deadline = &view["deadlines"][0];
        assert_eq!(deadline["title"], "Not open");
        assert_eq!(deadline["not_open"], true);
    })
    .await;
}

#[tokio::test]
async fn the_teaching_overview_is_empty_for_everyone_who_teaches_nothing_and_skips_archived_courses()
 {
    db_test(async |pool| {
        let w = world(pool).await;
        let task = w.task("Essay", "10", None, true).await;
        w.submit(&w.student, task).await;
        let empty =
            json!({"to_grade": [], "unpublished": [], "deadlines": [], "unpublished_content": []});
        assert_eq!(w.get(&w.student, "/me/teaching").await, empty);
        assert_eq!(w.get(&w.outsider, "/me/teaching").await, empty);
        assert_eq!(w.get(&w.outsider_student, "/me/teaching").await, empty);
        assert_eq!(
            w.get(&w.owner, "/me/teaching").await["to_grade"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        w.ok(
            "POST",
            &w.owner,
            &format!("/courses/{}/archive", w.course),
            None,
            200,
        )
        .await;
        assert_eq!(w.get(&w.owner, "/me/teaching").await, empty);
        assert_eq!(w.env.http.get("/me/teaching").send().await.status, 401);
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// /search
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn search_finds_courses_lectures_and_assignments_and_puts_titles_first() {
    db_test(async |pool| {
        let w = world(pool).await;
        sqlx::query!(
            "UPDATE teaching.courses SET description = 'Everything about recursion and trees' WHERE id = $1",
            w.course
        )
        .execute(&w.env.pool)
        .await
        .unwrap();
        w.item(Value::Null, "Notes", Some(doc("A long discussion of recursion in practice")), true).await;
        w.task("Recursion homework", "10", Some(FAR), true).await;

        let found = w.get(&w.student, "/search?q=recursion").await;
        let hits = found["items"].as_array().unwrap();
        assert_eq!(found["next_cursor"], Value::Null);
        assert_eq!(hits.len(), 3, "{found}");
        assert_eq!(hits[0]["title"], "Recursion homework", "a title match comes first");
        assert_eq!(hits[0]["kind"], "assignment");
        assert!(hits[0]["due_at"].is_string());
        let kinds: Vec<&str> = hits.iter().map(|h| h["kind"].as_str().unwrap()).collect();
        assert!(kinds.contains(&"course") && kinds.contains(&"lecture"));
        let lecture = hits.iter().find(|h| h["kind"] == "lecture").unwrap();
        assert!(lecture["snippet"].as_str().unwrap().contains("recursion"));
        assert_eq!(lecture["course_title"], "C");
        assert_eq!(lecture["status"], "published");
        let course = hits.iter().find(|h| h["kind"] == "course").unwrap();
        assert!(course["snippet"].as_str().unwrap().contains("recursion"));
        // Case does not matter.
        assert_eq!(w.get(&w.student, "/search?q=RECURSION").await["items"].as_array().unwrap().len(), 3);
    })
    .await;
}

#[tokio::test]
async fn a_student_searches_what_students_read_and_staff_search_the_working_copy() {
    db_test(async |pool| {
        let w = world(pool).await;
        let lecture = w
            .item(
                Value::Null,
                "Graphs",
                Some(doc("published words only")),
                true,
            )
            .await;
        w.ok(
            "PATCH",
            &w.owner,
            &format!("/items/{lecture}"),
            Some(json!({"title": "Graphs v2", "body": doc("unpublished secret words")})),
            200,
        )
        .await;
        w.item(
            Value::Null,
            "Draft chapter",
            Some(doc("more secret words")),
            false,
        )
        .await;

        let count = |v: &Value| v["items"].as_array().unwrap().len();
        assert_eq!(
            count(&w.get(&w.student, "/search?q=secret").await),
            0,
            "edits and drafts are not found"
        );
        let old = w.get(&w.student, "/search?q=published%20words").await;
        assert_eq!(old["items"][0]["title"], "Graphs", "the published title");
        assert_eq!(count(&w.get(&w.student, "/search?q=Graphs%20v2").await), 0);

        let staff = w.get(&w.owner, "/search?q=secret").await;
        assert_eq!(count(&staff), 2);
        let draft = staff["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h["title"] == "Draft chapter")
            .unwrap();
        assert_eq!(draft["status"], "draft");
        assert_eq!(count(&w.get(&w.owner, "/search?q=Graphs%20v2").await), 1);
    })
    .await;
}

#[tokio::test]
async fn search_never_leaves_the_callers_courses() {
    db_test(async |pool| {
        let w = world(pool).await;
        w.item(Value::Null, "Private lecture", Some(doc("zebra")), true)
            .await;
        for who in [&w.outsider, &w.outsider_student] {
            assert_eq!(w.get(who, "/search?q=zebra").await["items"], json!([]));
        }
        assert_eq!(
            w.get(&w.student, "/search?q=zebra").await["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        w.ok(
            "POST",
            &w.owner,
            &format!("/courses/{}/archive", w.course),
            None,
            200,
        )
        .await;
        assert_eq!(
            w.get(&w.student, "/search?q=zebra").await["items"]
                .as_array()
                .unwrap()
                .len(),
            1,
            "an archived course is still read"
        );
    })
    .await;
}

#[tokio::test]
async fn search_takes_the_text_as_it_is_and_wants_two_to_a_hundred_characters() {
    db_test(async |pool| {
        let w = world(pool).await;
        w.item(Value::Null, "100% sure", Some(doc("under_score")), true)
            .await;
        w.item(Value::Null, "Other", Some(doc("abcdef")), true)
            .await;
        let titles = |v: &Value| -> Vec<String> {
            v["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|h| h["title"].as_str().unwrap().to_owned())
                .collect()
        };
        assert_eq!(
            titles(&w.get(&w.student, "/search?q=0%25").await),
            ["100% sure"],
            "a percent sign is a percent sign"
        );
        assert_eq!(
            titles(&w.get(&w.student, "/search?q=r_s").await),
            ["100% sure"],
            "and an underscore too"
        );
        assert!(titles(&w.get(&w.student, "/search?q=a_c").await).is_empty());
        assert!(titles(&w.get(&w.student, "/search?q=%5C%5C").await).is_empty());

        for bad in ["", "a", "%20a%20"] {
            let response = w
                .env
                .call("GET", &w.student.token, &format!("/search?q={bad}"), None)
                .await;
            assert_eq!(response.status, 422, "{bad:?}");
        }
        let long = "x".repeat(101);
        assert_eq!(
            w.env
                .call("GET", &w.student.token, &format!("/search?q={long}"), None)
                .await
                .status,
            422
        );
        assert_eq!(
            w.env
                .call("GET", &w.student.token, "/search", None)
                .await
                .status,
            422
        );
        assert_eq!(w.env.http.get("/search?q=ab").send().await.status, 401);
    })
    .await;
}

#[tokio::test]
async fn a_snippet_is_cut_on_characters_and_marked() {
    db_test(async |pool| {
        let w = world(pool).await;
        let filler = "я".repeat(120);
        w.item(
            Value::Null,
            "Long",
            Some(doc(&format!("{filler} needle {filler}"))),
            true,
        )
        .await;
        let found = w.get(&w.student, "/search?q=NEEDLE").await;
        let snippet = found["items"][0]["snippet"].as_str().unwrap();
        assert!(
            snippet.starts_with('…') && snippet.ends_with('…'),
            "{snippet}"
        );
        assert!(snippet.contains("needle"));
        assert!(snippet.chars().count() < 140);
    })
    .await;
}
