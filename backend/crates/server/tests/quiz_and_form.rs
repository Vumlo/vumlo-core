//! ADR-0050 and ADR-0051: the `quiz` and `form` assignment types over HTTP.

mod common;

use ::common::Role;
use common::{Account, Env, db_test, id_of};
use serde_json::{Value, json};
use uuid::Uuid;

struct World {
    env: Env,
    owner: Account,
    student: Account,
    other: Account,
    third: Account,
    course: Uuid,
    module: Uuid,
}

async fn world(pool: sqlx::PgPool) -> World {
    let env = Env::new(pool).await;
    let owner = env.user("owner@lms.test", Role::Teacher).await;
    let student = env.user("student@lms.test", Role::Student).await;
    let other = env.user("other@lms.test", Role::Student).await;
    let third = env.user("third@lms.test", Role::Student).await;
    let (course, module) = env.course(&owner, &[&student, &other, &third]).await;
    World {
        env,
        owner,
        student,
        other,
        third,
        course,
        module,
    }
}

fn doc(text: &str) -> Value {
    json!({"v": 1, "attachments": [], "blocks": [
        {"id": "p1", "type": "paragraph", "content": [{"type": "text", "text": text}]}]})
}

fn opt(id: &str) -> Value {
    json!({"id": id, "content": doc(&format!("option {id}"))})
}

/// Six points: a single choice (2), a multiple choice with partial credit (3), a number (1).
fn quiz_config() -> Value {
    json!({
        "v": 1, "shuffle_questions": false, "show_correct": true,
        "questions": [
            {"id": "q1", "type": "single_choice", "points": "2", "prompt": doc("one"),
             "options": [opt("a"), opt("b"), opt("c")], "correct": ["b"],
             "explanation": doc("because b"), "grading_notes": "SECRET-NOTE"},
            {"id": "q2", "type": "multiple_choice", "points": "3", "prompt": doc("two"),
             "options": [opt("a"), opt("b"), opt("c"), opt("d")], "correct": ["a", "b"],
             "partial_credit": true},
            {"id": "q3", "type": "number", "points": "1", "prompt": doc("three"),
             "answer": "3.14", "tolerance": "0.01"},
        ]
    })
}

/// The same quiz with a free-text question worth four more points.
fn quiz_with_essay() -> Value {
    let mut config = quiz_config();
    config["questions"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": "q4", "type": "long_text", "points": "4", "prompt": doc("explain")}));
    config
}

fn right() -> Value {
    json!({"q1": ["b"], "q2": ["a", "b"], "q3": "3,14"})
}

fn wrong() -> Value {
    json!({"q1": ["a"], "q2": ["c", "d"], "q3": "2"})
}

fn form_config() -> Value {
    json!({
        "v": 1, "note": "Register your team",
        "fields": [
            {"id": "name", "type": "short", "label": "Team name", "required": true,
             "rules": {"max_length": 30}},
            {"id": "topic", "type": "choice", "label": "Topic", "required": true,
             "options": [{"id": "t1", "text": "Graphs"}, {"id": "t2", "text": "Trees"}],
             "rules": {"exclusive": true}},
            {"id": "team", "type": "people", "label": "Members", "required": false,
             "rules": {"min": 0, "max": 2}},
            {"id": "repo", "type": "link", "label": "Repository", "required": false},
        ]
    })
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

    async fn create(&self, kind: &str, config: Value, points: &str) -> common::Resp {
        self.call(
            "POST",
            &self.owner,
            &format!("/modules/{}/items", self.module),
            Some(
                json!({"kind": "assignment", "title": "Task", "assignment": {
                "type": kind, "config": config, "max_points": points}}),
            ),
        )
        .await
    }

    /// A published assignment of the type.
    async fn make(&self, kind: &str, config: Value, points: &str) -> Uuid {
        let created = self.create(kind, config, points).await;
        assert_eq!(
            created.status,
            201,
            "{}",
            String::from_utf8_lossy(&created.body)
        );
        let item = id_of(&created.json());
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

    async fn item(&self, who: &Account, item: Uuid) -> Value {
        self.ok("GET", who, &format!("/items/{item}"), None, 200)
            .await
    }

    async fn draft(&self, who: &Account, item: Uuid, answers: Value) -> common::Resp {
        self.call(
            "PUT",
            who,
            &format!("/assignments/{item}/submission/draft"),
            Some(json!({"content": {"answers": answers}})),
        )
        .await
    }

    async fn submit(&self, who: &Account, item: Uuid) -> common::Resp {
        self.call(
            "POST",
            who,
            &format!("/assignments/{item}/submission/submit"),
            None,
        )
        .await
    }

    /// Saves the answers as a draft and submits them; returns the submit.
    async fn take(&self, who: &Account, item: Uuid, answers: Value) -> common::Resp {
        let saved = self.draft(who, item, answers).await;
        assert_eq!(
            saved.status,
            200,
            "{}",
            String::from_utf8_lossy(&saved.body)
        );
        self.submit(who, item).await
    }

    async fn patch(&self, item: Uuid, body: Value) -> common::Resp {
        self.call("PATCH", &self.owner, &format!("/items/{item}"), Some(body))
            .await
    }

    async fn publish_grades(&self, item: Uuid) {
        self.ok(
            "POST",
            &self.owner,
            &format!("/assignments/{item}/grades/publish"),
            Some(json!({"students": "all"})),
            200,
        )
        .await;
    }

    async fn draft_grade(&self, item: Uuid, student: &Account) -> Option<(String, Option<Uuid>)> {
        sqlx::query!(
            "SELECT draft_score, draft_updated_by FROM gradebook.grades
             WHERE assignment_id = $1 AND student_id = $2",
            item,
            student.id
        )
        .fetch_optional(&self.env.pool)
        .await
        .unwrap()
        .map(|row| {
            (
                format!("{:.2}", row.draft_score.unwrap()),
                row.draft_updated_by,
            )
        })
    }
}

fn field_of(response: &common::Resp, status: u16) -> String {
    let problem = response.problem(status);
    problem
        .errors
        .first()
        .map(|error| error.field.clone())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// quiz
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_quiz_is_checked_when_it_is_written() {
    db_test(async |pool| {
        let w = world(pool).await;
        let created = w.create("quiz", quiz_config(), "6").await;
        assert_eq!(
            created.status,
            201,
            "{}",
            String::from_utf8_lossy(&created.body)
        );

        let mismatch = w.create("quiz", quiz_config(), "20").await;
        assert_eq!(field_of(&mismatch, 422), "assignment.max_points");
        let message = &mismatch.problem(422).errors[0].message;
        assert!(message.contains("6.00"), "{message}");

        let mut broken = quiz_config();
        broken["questions"][0]["correct"] = json!(["zzz"]);
        assert_eq!(
            field_of(&w.create("quiz", broken, "6").await, 422),
            "assignment.config"
        );

        let mut extra = quiz_config();
        extra["questions"][2]["options"] = json!([opt("a"), opt("b")]);
        assert_eq!(
            field_of(&w.create("quiz", extra, "6").await, 422),
            "assignment.config"
        );

        let mut prompt = quiz_config();
        prompt["questions"][0]["prompt"] = json!("a plain string");
        let refused = w.create("quiz", prompt, "6").await;
        assert!(
            refused.problem(422).errors[0]
                .message
                .contains("questions[0].prompt")
        );

        // The points are the sum of the questions also when they are changed later.
        let item = id_of(&created.json());
        assert_eq!(
            field_of(&w.patch(item, json!({"max_points": "7"})).await, 422),
            "max_points"
        );
        let mut bigger = quiz_config();
        bigger["questions"][2]["points"] = json!("2");
        assert_eq!(
            field_of(&w.patch(item, json!({"config": bigger.clone()})).await, 422),
            "max_points"
        );
        let both = w
            .patch(item, json!({"config": bigger, "max_points": "7"}))
            .await;
        assert_eq!(both.status, 200, "{}", String::from_utf8_lossy(&both.body));
    })
    .await;
}

#[tokio::test]
async fn a_student_never_receives_the_answer_key() {
    db_test(async |pool| {
        let w = world(pool).await;
        let item = w.make("quiz", quiz_config(), "6").await;

        let staff = w.item(&w.owner, item).await.to_string();
        for key in [
            "\"correct\"",
            "\"answer\"",
            "\"tolerance\"",
            "\"explanation\"",
            "\"partial_credit\"",
            "SECRET-NOTE",
        ] {
            assert!(staff.contains(key), "staff should see {key}");
        }
        let seen = w.item(&w.student, item).await;
        let raw = seen.to_string();
        for key in [
            "\"correct\"",
            "\"answer\"",
            "\"tolerance\"",
            "\"accepted\"",
            "\"explanation\"",
            "\"partial_credit\"",
            "SECRET-NOTE",
            "because b",
        ] {
            assert!(!raw.contains(key), "a student must not see {key}: {raw}");
        }
        assert_eq!(
            seen["assignment"]["config"]["questions"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert!(seen["assignment"]["config"]["questions"][0]["options"].is_array());

        // Nor from the other reads that carry the config: the answers, the result, the list.
        w.take(&w.student, item, right()).await;
        let mine = w
            .ok(
                "GET",
                &w.student,
                &format!("/assignments/{item}/submission"),
                None,
                200,
            )
            .await
            .to_string();
        assert!(
            !mine.contains("auto_scores") && !mine.contains("SECRET"),
            "{mine}"
        );
    })
    .await;
}

#[tokio::test]
async fn a_submitted_quiz_is_scored_and_becomes_a_draft_grade_without_a_person() {
    db_test(async |pool| {
        let w = world(pool).await;
        let item = w.make("quiz", quiz_config(), "6").await;

        let attempt = w.take(&w.student, item, right()).await;
        assert_eq!(
            attempt.status,
            201,
            "{}",
            String::from_utf8_lossy(&attempt.body)
        );
        assert_eq!(
            w.draft_grade(item, &w.student).await,
            Some(("6.00".into(), None))
        );
        assert_eq!(w.draft_grade(item, &w.other).await, None);

        // Staff read the score of each question with the attempt.
        let attempts = w
            .ok(
                "GET",
                &w.owner,
                &format!("/assignments/{item}/submissions/{}", w.student.id),
                None,
                200,
            )
            .await;
        assert_eq!(
            attempts["items"][0]["auto_scores"],
            json!({"q1": "2.00", "q2": "3.00", "q3": "1.00"})
        );
        // The history says the server did it, and nobody is named.
        let history = w
            .ok(
                "GET",
                &w.owner,
                &format!("/assignments/{item}/grades/{}/history", w.student.id),
                None,
                200,
            )
            .await;
        assert_eq!(history["items"][0]["action"], "auto_score");
        assert_eq!(history["items"][0]["actor"], Value::Null);
        assert_eq!(history["items"][0]["score"], "6.00");

        // Nothing is published: the student sees no score yet (AGENTS.md section 6).
        let mine = w
            .ok(
                "GET",
                &w.student,
                &format!("/courses/{}/my-grades", w.course),
                None,
                200,
            )
            .await;
        assert!(
            mine.to_string().contains("\"score\":null") || !mine.to_string().contains("6.00"),
            "{mine}"
        );

        // A second attempt replaces the draft score and leaves the earlier one in the history.
        let second = w.take(&w.student, item, wrong()).await;
        assert_eq!(second.status, 201);
        assert_eq!(
            w.draft_grade(item, &w.student).await,
            Some(("0.00".into(), None))
        );
        let history = w
            .ok(
                "GET",
                &w.owner,
                &format!("/assignments/{item}/grades/{}/history", w.student.id),
                None,
                200,
            )
            .await;
        assert_eq!(history["items"].as_array().unwrap().len(), 2);
    })
    .await;
}

#[tokio::test]
async fn partial_credit_numbers_and_unanswered_questions_are_scored() {
    db_test(async |pool| {
        let w = world(pool).await;
        let item = w.make("quiz", quiz_config(), "6").await;
        w.take(
            &w.student,
            item,
            json!({"q1": ["b"], "q2": ["a"], "q3": "3.15"}),
        )
        .await;
        let attempts = w
            .ok(
                "GET",
                &w.owner,
                &format!("/assignments/{item}/submissions/{}", w.student.id),
                None,
                200,
            )
            .await;
        assert_eq!(
            attempts["items"][0]["auto_scores"],
            json!({"q1": "2.00", "q2": "1.50", "q3": "1.00"})
        );
        assert_eq!(w.draft_grade(item, &w.student).await.unwrap().0, "4.50");

        w.take(&w.other, item, json!({"q3": "1"})).await;
        assert_eq!(w.draft_grade(item, &w.other).await.unwrap().0, "0.00");
    })
    .await;
}

#[tokio::test]
async fn a_quiz_with_a_free_text_question_is_scored_but_gets_no_draft_grade() {
    db_test(async |pool| {
        let w = world(pool).await;
        let item = w.make("quiz", quiz_with_essay(), "10").await;
        let mut answers = right();
        answers["q4"] = doc("my explanation");
        let attempt = w.take(&w.student, item, answers).await;
        assert_eq!(
            attempt.status,
            201,
            "{}",
            String::from_utf8_lossy(&attempt.body)
        );
        assert_eq!(
            w.draft_grade(item, &w.student).await,
            None,
            "a teacher grades the whole by hand"
        );
        let attempts = w
            .ok(
                "GET",
                &w.owner,
                &format!("/assignments/{item}/submissions/{}", w.student.id),
                None,
                200,
            )
            .await;
        assert_eq!(
            attempts["items"][0]["auto_scores"],
            json!({"q1": "2.00", "q2": "3.00", "q3": "1.00"}),
            "the free text has no score of its own"
        );
        // The teacher grades it like any assignment.
        w.ok(
            "PUT",
            &w.owner,
            &format!("/assignments/{item}/grades/{}", w.student.id),
            Some(json!({"score": "9", "feedback": "Good essay"})),
            200,
        )
        .await;
        assert_eq!(w.draft_grade(item, &w.student).await.unwrap().0, "9.00");
    })
    .await;
}

#[tokio::test]
async fn answers_are_checked_while_saving_and_a_submit_needs_an_answer() {
    db_test(async |pool| {
        let w = world(pool).await;
        let item = w.make("quiz", quiz_config(), "6").await;

        for (answers, path) in [
            (json!({"zz": ["a"]}), "content.answers.zz"),
            (json!({"q1": "b"}), "content.answers.q1"),
            (json!({"q1": ["nope"]}), "content.answers.q1"),
            (json!({"q1": ["a", "b"]}), "content.answers.q1"),
            (json!({"q3": "many"}), "content.answers.q3"),
        ] {
            let refused = w.draft(&w.student, item, answers.clone()).await;
            assert_eq!(field_of(&refused, 422), path, "{answers}");
        }
        // An empty draft saves; submitting it does not.
        assert_eq!(w.draft(&w.student, item, json!({})).await.status, 200);
        assert_eq!(field_of(&w.submit(&w.student, item).await, 422), "content");
        assert_eq!(w.draft_grade(item, &w.student).await, None);
        // A document is not a quiz answer.
        let document = w
            .call(
                "PUT",
                &w.student,
                &format!("/assignments/{item}/submission/draft"),
                Some(json!({"content": doc("text")})),
            )
            .await;
        assert_eq!(document.status, 422);
    })
    .await;
}

#[tokio::test]
async fn the_result_appears_with_the_published_grade_and_shows_the_key_only_when_asked() {
    db_test(async |pool| {
        let w = world(pool).await;
        let item = w.make("quiz", quiz_config(), "6").await;
        let result = format!("/assignments/{item}/result");
        w.take(
            &w.student,
            item,
            json!({"q1": ["a"], "q2": ["a", "b"], "q3": "3.14"}),
        )
        .await;

        // Scored but not published: there is nothing to see.
        assert_eq!(w.call("GET", &w.student, &result, None).await.status, 404);
        w.publish_grades(item).await;

        let shown = w.ok("GET", &w.student, &result, None, 200).await;
        assert_eq!(shown["score"], "4.00");
        assert_eq!(shown["max_points"], "6.00");
        assert_eq!(shown["attempt"], 1);
        assert_eq!(shown["feedback"], Value::Null);
        let questions = shown["questions"].as_array().unwrap();
        assert_eq!(questions.len(), 3);
        assert_eq!(questions[0]["got"], "0.00");
        assert_eq!(questions[0]["answer"], json!(["a"]));
        assert_eq!(questions[0]["correct"], json!(["b"]));
        assert!(questions[0]["explanation"].is_object());
        assert_eq!(questions[1]["got"], "3.00");
        assert_eq!(questions[2]["correct"], json!("3.14"));

        // A teacher's change of the published value shows as the score; the question scores stay.
        w.ok(
            "PUT",
            &w.owner,
            &format!("/assignments/{item}/grades/{}", w.student.id),
            Some(json!({"score": "5", "feedback": "Rounded up"})),
            200,
        )
        .await;
        let before = w.ok("GET", &w.student, &result, None, 200).await;
        assert_eq!(
            before["score"], "4.00",
            "a draft change is not seen before it is published"
        );
        w.publish_grades(item).await;
        let after = w.ok("GET", &w.student, &result, None, 200).await;
        assert_eq!(after["score"], "5.00");
        assert_eq!(after["feedback"], "Rounded up");
        assert_eq!(after["questions"][0]["got"], "0.00");

        // Switching the answers off hides the key and the explanations.
        let mut hidden = quiz_config();
        hidden["show_correct"] = json!(false);
        assert_eq!(w.patch(item, json!({"config": hidden})).await.status, 200);
        let shown = w.ok("GET", &w.student, &result, None, 200).await;
        let raw = shown.to_string();
        assert!(
            !raw.contains("\"correct\":[") && !raw.contains("because b"),
            "{raw}"
        );
        assert_eq!(shown["questions"][0]["correct"], Value::Null);

        // Nobody else reads it: a student who did not take the quiz, and the teacher.
        assert_eq!(w.call("GET", &w.other, &result, None).await.status, 404);
        assert_eq!(w.call("GET", &w.owner, &result, None).await.status, 403);
        let anonymous = w.env.http.get(&result).send().await;
        assert_eq!(anonymous.status, 401);
    })
    .await;
}

#[tokio::test]
async fn the_questions_are_frozen_after_the_first_attempt_but_the_wording_is_not() {
    db_test(async |pool| {
        let w = world(pool).await;
        let item = w.make("quiz", quiz_config(), "6").await;

        // Before any attempt everything may change.
        let mut other_key = quiz_config();
        other_key["questions"][0]["correct"] = json!(["c"]);
        assert_eq!(
            w.patch(item, json!({"config": other_key})).await.status,
            200
        );

        w.take(&w.student, item, right()).await;

        let mut key = quiz_config();
        key["questions"][0]["correct"] = json!(["a"]);
        assert_eq!(w.patch(item, json!({"config": key})).await.status, 409);
        let mut shorter = quiz_config();
        shorter["questions"].as_array_mut().unwrap().pop();
        assert_eq!(
            w.patch(item, json!({"config": shorter, "max_points": "5"}))
                .await
                .status,
            409
        );
        let mut points = quiz_config();
        points["questions"][0]["points"] = json!("3");
        assert_eq!(
            w.patch(item, json!({"config": points, "max_points": "7"}))
                .await
                .status,
            409
        );
        let mut answer = quiz_config();
        answer["questions"][2]["answer"] = json!("3.15");
        assert_eq!(w.patch(item, json!({"config": answer})).await.status, 409);
        let mut order = quiz_config();
        order["questions"].as_array_mut().unwrap().reverse();
        assert_eq!(w.patch(item, json!({"config": order})).await.status, 409);

        // Wording, the notes, the explanation and the two switches may change.
        let mut wording = quiz_config();
        wording["questions"][0]["correct"] = json!(["c"]);
        wording["questions"][0]["prompt"] = doc("a clearer question");
        wording["questions"][0]["explanation"] = doc("a better explanation");
        wording["questions"][0]["grading_notes"] = json!("new notes");
        wording["questions"][0]["options"][0]["content"] = doc("reworded");
        wording["shuffle_questions"] = json!(true);
        wording["show_correct"] = json!(false);
        let saved = w.patch(item, json!({"config": wording})).await;
        assert_eq!(
            saved.status,
            200,
            "{}",
            String::from_utf8_lossy(&saved.body)
        );
        // The frozen part is what was stored when the first attempt came in (the key `c`).
        let stored = w.item(&w.owner, item).await;
        assert_eq!(
            stored["assignment"]["config"]["questions"][0]["correct"],
            json!(["c"])
        );
    })
    .await;
}

#[tokio::test]
async fn nobody_else_can_use_the_quiz_endpoints_on_the_wrong_type() {
    db_test(async |pool| {
        let w = world(pool).await;
        let essay = w.env.assignment(&w.owner, w.module, "10").await;
        let form = w.make("form", form_config(), "5").await;
        let quiz = w.make("quiz", quiz_config(), "6").await;
        for item in [essay, form] {
            assert_eq!(
                w.call(
                    "GET",
                    &w.student,
                    &format!("/assignments/{item}/result"),
                    None
                )
                .await
                .status,
                404
            );
        }
        for item in [essay, quiz] {
            assert_eq!(
                w.call(
                    "GET",
                    &w.owner,
                    &format!("/assignments/{item}/responses"),
                    None
                )
                .await
                .status,
                404
            );
        }
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// form
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_form_is_checked_when_it_is_written() {
    db_test(async |pool| {
        let w = world(pool).await;
        // A form is graded by hand, so it has points like any other assignment.
        assert_eq!(w.create("form", form_config(), "5").await.status, 201);
        assert_eq!(
            field_of(&w.create("form", form_config(), "0").await, 422),
            "assignment.max_points"
        );

        let mut unknown = form_config();
        unknown["fields"][0]["type"] = json!("rainbow");
        assert_eq!(
            field_of(&w.create("form", unknown, "5").await, 422),
            "assignment.config"
        );
        let mut rules = form_config();
        rules["fields"][0]["rules"] = json!({"exclusive": true});
        assert_eq!(
            field_of(&w.create("form", rules, "5").await, 422),
            "assignment.config"
        );
        let mut options = form_config();
        options["fields"][1]["options"] = json!([{"id": "t1", "text": "Only"}]);
        assert_eq!(
            field_of(&w.create("form", options, "5").await, 422),
            "assignment.config"
        );
        assert_eq!(
            field_of(
                &w.create("form", json!({"v": 1, "fields": []}), "5").await,
                422
            ),
            "assignment.config"
        );
    })
    .await;
}

#[tokio::test]
async fn a_form_needs_its_required_fields_only_when_it_is_submitted() {
    db_test(async |pool| {
        let w = world(pool).await;
        let item = w.make("form", form_config(), "5").await;

        assert_eq!(
            w.draft(&w.student, item, json!({"repo": "https://example.org/r"}))
                .await
                .status,
            200
        );
        let incomplete = w.submit(&w.student, item).await;
        assert_eq!(incomplete.status, 422);
        let fields: Vec<String> = incomplete
            .problem(422)
            .errors
            .iter()
            .map(|error| error.field.clone())
            .collect();
        assert_eq!(fields, ["content.answers.name", "content.answers.topic"]);

        for (answers, path) in [
            (json!({"name": "x".repeat(31)}), "content.answers.name"),
            (json!({"topic": ["t9"]}), "content.answers.topic"),
            (json!({"repo": "ftp://example.org"}), "content.answers.repo"),
            (json!({"nope": "x"}), "content.answers.nope"),
        ] {
            assert_eq!(
                field_of(&w.draft(&w.student, item, answers.clone()).await, 422),
                path,
                "{answers}"
            );
        }

        let done = w
            .take(
                &w.student,
                item,
                json!({"name": "Team A", "topic": ["t1"], "repo": "https://example.org/r"}),
            )
            .await;
        assert_eq!(done.status, 201, "{}", String::from_utf8_lossy(&done.body));
        assert_eq!(
            w.draft_grade(item, &w.student).await,
            None,
            "a form is graded by hand"
        );
        assert_eq!(
            w.call(
                "GET",
                &w.student,
                &format!("/assignments/{item}/result"),
                None
            )
            .await
            .status,
            404
        );
    })
    .await;
}

#[tokio::test]
async fn an_exclusive_option_goes_to_the_first_student_and_students_do_not_see_who() {
    db_test(async |pool| {
        let w = world(pool).await;
        let item = w.make("form", form_config(), "5").await;
        let options = |seen: &Value| seen["assignment"]["config"]["fields"][1]["options"].clone();

        let first = w
            .take(&w.student, item, json!({"name": "A", "topic": ["t1"]}))
            .await;
        assert_eq!(first.status, 201);

        let clash = w
            .take(&w.other, item, json!({"name": "B", "topic": ["t1"]}))
            .await;
        assert_eq!(
            clash.status,
            409,
            "{}",
            String::from_utf8_lossy(&clash.body)
        );
        // Their draft is kept; they pick another option and go on.
        let free = w
            .take(&w.other, item, json!({"name": "B", "topic": ["t2"]}))
            .await;
        assert_eq!(free.status, 201);

        // The third student sees both taken, and not by whom.
        let seen = w.item(&w.third, item).await;
        assert_eq!(options(&seen)[0]["taken"], true);
        assert_eq!(options(&seen)[1]["taken"], true);
        let raw = seen.to_string();
        assert!(
            !raw.contains("taken_by") && !raw.contains("has_answers"),
            "{raw}"
        );
        // The first student sees their own option as theirs to keep.
        let mine = w.item(&w.student, item).await;
        assert_eq!(options(&mine)[0]["taken"], false);
        assert_eq!(options(&mine)[1]["taken"], true);
        // The same student can submit again with the option they hold.
        assert_eq!(
            w.take(&w.student, item, json!({"name": "A2", "topic": ["t1"]}))
                .await
                .status,
            201
        );

        // Staff see who holds it, and which fields have answers.
        let staff = w.item(&w.owner, item).await;
        let fields = &staff["assignment"]["config"]["fields"];
        assert!(
            fields[1]["options"][0]["taken_by"]
                .as_str()
                .unwrap()
                .contains("Test User"),
            "{staff}"
        );
        assert_eq!(fields[0]["has_answers"], true);
        assert_eq!(fields[3]["has_answers"], false);
    })
    .await;
}

#[tokio::test]
async fn two_students_cannot_both_win_an_exclusive_option() {
    db_test(async |pool| {
        let w = world(pool).await;
        let item = w.make("form", form_config(), "5").await;
        for who in [&w.student, &w.other] {
            let saved = w
                .draft(who, item, json!({"name": "T", "topic": ["t1"]}))
                .await;
            assert_eq!(saved.status, 200);
        }
        let (a, b) = tokio::join!(w.submit(&w.student, item), w.submit(&w.other, item));
        let mut statuses = [a.status, b.status];
        statuses.sort_unstable();
        assert_eq!(statuses, [201, 409]);
    })
    .await;
}

#[tokio::test]
async fn the_people_of_a_form_are_current_students_other_than_the_submitter() {
    db_test(async |pool| {
        let w = world(pool).await;
        let item = w.make("form", form_config(), "5").await;
        let base = json!({"name": "A", "topic": ["t1"]});
        let with = |people: Value| {
            let mut answers = base.clone();
            answers["team"] = people;
            answers
        };

        // The teacher is no student; the submitter is not their own teammate.
        for people in [
            json!([w.owner.id]),
            json!([w.student.id]),
            json!([Uuid::now_v7()]),
        ] {
            let refused = w.take(&w.student, item, with(people.clone())).await;
            assert_eq!(refused.status, 422, "{people}");
        }
        // Too many people are refused while saving.
        let three = json!([w.other.id, w.third.id, Uuid::now_v7()]);
        assert_eq!(
            field_of(&w.draft(&w.student, item, with(three)).await, 422),
            "content.answers.team"
        );
        let ok = w
            .take(&w.student, item, with(json!([w.other.id, w.third.id])))
            .await;
        assert_eq!(ok.status, 201, "{}", String::from_utf8_lossy(&ok.body));
    })
    .await;
}

#[tokio::test]
async fn staff_read_the_responses_to_a_form_and_see_who_has_not_answered() {
    db_test(async |pool| {
        let w = world(pool).await;
        let item = w.make("form", form_config(), "5").await;
        let path = format!("/assignments/{item}/responses");

        let empty = w.ok("GET", &w.owner, &path, None, 200).await;
        assert_eq!(empty["total_students"], 3);
        assert_eq!(empty["rows"], json!([]));
        assert_eq!(empty["missing"].as_array().unwrap().len(), 3);

        w.take(
            &w.student,
            item,
            json!({"name": "First try", "topic": ["t1"]}),
        )
        .await;
        w.take(
            &w.student,
            item,
            json!({"name": "Second try", "topic": ["t1"], "repo": "https://example.org/r"}),
        )
        .await;
        w.take(&w.other, item, json!({"name": "B", "topic": ["t2"]}))
            .await;
        // A draft is not a response.
        w.draft(&w.third, item, json!({"name": "unfinished"})).await;

        let seen = w.ok("GET", &w.owner, &path, None, 200).await;
        assert_eq!(seen["total_students"], 3);
        let rows = seen["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        let mine = rows
            .iter()
            .find(|row| row["student_id"] == json!(w.student.id))
            .unwrap();
        assert_eq!(mine["attempt"], 2, "the latest attempt");
        assert_eq!(mine["answers"]["name"], "Second try");
        assert_eq!(mine["answers"]["repo"], "https://example.org/r");
        assert_eq!(mine["is_late"], false);
        let missing = seen["missing"].as_array().unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0]["user_id"], json!(w.third.id));
        assert!(!seen.to_string().contains("unfinished"));

        // Students do not read them.
        assert_eq!(w.call("GET", &w.student, &path, None).await.status, 403);
        assert_eq!(w.env.http.get(&path).send().await.status, 401);
    })
    .await;
}

#[tokio::test]
async fn the_fields_of_a_form_are_frozen_after_the_first_attempt_and_computed_facts_are_not_stored()
{
    db_test(async |pool| {
        let w = world(pool).await;
        let item = w.make("form", form_config(), "5").await;
        w.take(&w.student, item, json!({"name": "A", "topic": ["t1"]}))
            .await;

        // What staff read can be sent back as it is: the facts are dropped, not stored.
        let read = w.item(&w.owner, item).await["assignment"]["config"].clone();
        assert!(read["fields"][1]["options"][0].get("taken_by").is_some());
        assert_eq!(
            w.patch(item, json!({"config": read.clone()})).await.status,
            200
        );
        let stored: i64 = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM content.assignments
               WHERE item_id = $1 AND config::text ~ 'taken|has_answers'"#,
            item
        )
        .fetch_one(&w.env.pool)
        .await
        .unwrap();
        assert_eq!(stored, 0);

        let mut renamed = form_config();
        renamed["fields"][0]["id"] = json!("title");
        assert_eq!(w.patch(item, json!({"config": renamed})).await.status, 409);
        let mut optional = form_config();
        optional["fields"][0]["required"] = json!(false);
        assert_eq!(w.patch(item, json!({"config": optional})).await.status, 409);
        let mut rule = form_config();
        rule["fields"][0]["rules"] = json!({"max_length": 40});
        assert_eq!(w.patch(item, json!({"config": rule})).await.status, 409);

        let mut wording = form_config();
        wording["note"] = json!("A new note");
        wording["fields"][0]["label"] = json!("Name of your team");
        wording["fields"][1]["options"][0]["text"] = json!("Graph theory");
        let saved = w.patch(item, json!({"config": wording})).await;
        assert_eq!(
            saved.status,
            200,
            "{}",
            String::from_utf8_lossy(&saved.body)
        );
    })
    .await;
}
