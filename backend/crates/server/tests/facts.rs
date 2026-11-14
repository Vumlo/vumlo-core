//! ADR-0093: `content.facts`, the copy of what a student did that the access rules read. The
//! subscribers of `submission.submitted` and `grade.publication_changed` keep it up to date in the
//! transaction of the change, and `learning::rebuild_facts` makes it again from the sources.

mod common;

use ::common::Role;
use chrono::{DateTime, Duration, Utc};
use common::{Account, Env, db_test, essay, id_of, spawn_app, test_settings};
use content::{FactScope, SourceFact, replace_source_facts};
use learning::schedule::tick;
use serde_json::{Value, json};
use server::app::AppState;
use sqlx::PgPool;
use uuid::Uuid;

struct World {
    env: Env,
    state: AppState,
    teacher: Account,
    student: Account,
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
    let (_, module) = env.course(&teacher, &[&student]).await;
    World {
        env,
        state,
        teacher,
        student,
        module,
    }
}

type Row = (Uuid, Option<DateTime<Utc>>, Option<String>, bool);

impl World {
    async fn task(&self) -> Uuid {
        self.env.assignment(&self.teacher, self.module, "10").await
    }

    async fn go(&self, who: &Account, method: &str, path: &str, body: Option<Value>, ok: u16) {
        self.env.ok(method, &who.token, path, body, ok).await;
    }

    /// The student saves a text answer and submits it.
    async fn submit(&self, assignment: Uuid) {
        self.go(
            &self.student,
            "PUT",
            &format!("/assignments/{assignment}/submission/draft"),
            Some(json!({"content": essay("answer")})),
            200,
        )
        .await;
        self.go(
            &self.student,
            "POST",
            &format!("/assignments/{assignment}/submission/submit"),
            None,
            201,
        )
        .await;
    }

    /// A published text item.
    async fn lecture(&self) -> Uuid {
        let item = id_of(
            &self
                .env
                .ok(
                    "POST",
                    &self.teacher.token,
                    &format!("/modules/{}/items", self.module),
                    Some(json!({"kind": "text", "title": "L", "body": essay("read")})),
                    201,
                )
                .await,
        );
        self.go(
            &self.teacher,
            "POST",
            &format!("/items/{item}/publish"),
            None,
            200,
        )
        .await;
        item
    }

    /// The student marks a text item done.
    async fn mark(&self, item: Uuid) {
        self.go(
            &self.student,
            "PUT",
            &format!("/items/{item}/completion"),
            None,
            204,
        )
        .await;
    }

    /// The teacher saves a draft grade; `None` clears it.
    async fn draft(&self, assignment: Uuid, points: Option<&str>) {
        self.go(
            &self.teacher,
            "PUT",
            &format!("/assignments/{assignment}/grades/{}", self.student.id),
            Some(json!({"score": points, "feedback": null})),
            200,
        )
        .await;
    }

    async fn publish_grades(&self, assignment: Uuid) {
        self.go(
            &self.teacher,
            "POST",
            &format!("/assignments/{assignment}/grades/publish"),
            Some(json!({"students": "all"})),
            200,
        )
        .await;
    }

    async fn grade(&self, assignment: Uuid, points: Option<&str>) {
        self.draft(assignment, points).await;
        self.publish_grades(assignment).await;
    }

    /// The student's row for the item: first submit, published score, whether it is marked.
    async fn fact(&self, item: Uuid) -> Option<(Option<DateTime<Utc>>, Option<String>, bool)> {
        sqlx::query!(
            r#"SELECT submitted_at, published_score::text AS score, marked_at IS NOT NULL AS "marked!"
               FROM content.facts WHERE item_id = $1 AND student_id = $2"#,
            item,
            self.student.id
        )
        .fetch_optional(&self.env.pool)
        .await
        .unwrap()
        .map(|row| (row.submitted_at, row.score, row.marked))
    }

    /// Every row, in a fixed order.
    async fn snapshot(&self) -> Vec<Row> {
        sqlx::query!(
            r#"SELECT item_id, submitted_at, published_score::text AS score,
                      marked_at IS NOT NULL AS "marked!"
               FROM content.facts ORDER BY item_id, student_id"#
        )
        .fetch_all(&self.env.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.item_id, row.submitted_at, row.score, row.marked))
        .collect()
    }

    async fn first_attempt_at(&self, assignment: Uuid) -> DateTime<Utc> {
        sqlx::query_scalar!(
            r#"SELECT submitted_at AS "at!" FROM assessment.submissions
               WHERE assignment_id = $1 AND student_id = $2 AND attempt = 1"#,
            assignment,
            self.student.id
        )
        .fetch_one(&self.env.pool)
        .await
        .unwrap()
    }
}

#[tokio::test]
async fn a_submit_sets_the_first_time_and_a_later_attempt_keeps_it() {
    db_test(async |pool| {
        let w = world(pool).await;
        let task = w.task().await;
        assert_eq!(w.fact(task).await, None);

        w.submit(task).await;
        let (first, score, marked) = w.fact(task).await.unwrap();
        assert_eq!(first, Some(w.first_attempt_at(task).await));
        assert_eq!((score, marked), (None, false));

        w.submit(task).await;
        assert_eq!(
            w.fact(task).await.unwrap().0,
            first,
            "the first attempt counts"
        );
    })
    .await;
}

#[tokio::test]
async fn publishing_a_grade_sets_the_score_and_publishing_again_changes_it() {
    db_test(async |pool| {
        let w = world(pool).await;
        let task = w.task().await;

        w.grade(task, Some("7")).await;
        assert_eq!(w.fact(task).await.unwrap().1.as_deref(), Some("7.00"));

        w.grade(task, Some("9.5")).await;
        assert_eq!(w.fact(task).await.unwrap().1.as_deref(), Some("9.50"));
    })
    .await;
}

#[tokio::test]
async fn taking_a_grade_back_clears_the_score_and_keeps_the_submit() {
    db_test(async |pool| {
        let w = world(pool).await;
        let task = w.task().await;
        w.submit(task).await;
        w.grade(task, Some("7")).await;
        let events = async |kind: &str| {
            sqlx::query_scalar!(
                r#"SELECT count(*) AS "n!" FROM events.outbox WHERE type = $1"#,
                kind
            )
            .fetch_one(&w.env.pool)
            .await
            .unwrap()
        };
        let (changed, published) = (
            events("grade.publication_changed").await,
            events("grade.published").await,
        );

        w.grade(task, None).await;

        let (submitted, score, _) = w.fact(task).await.unwrap();
        assert!(submitted.is_some());
        assert_eq!(score, None);
        // The facts heard of it; the plugins were told nothing, as there is no value to tell.
        assert_eq!(events("grade.publication_changed").await, changed + 1);
        assert_eq!(events("grade.published").await, published);
    })
    .await;
}

#[tokio::test]
async fn a_draft_grade_never_reaches_the_facts() {
    db_test(async |pool| {
        let w = world(pool).await;
        let task = w.task().await;

        w.draft(task, Some("8")).await;

        assert_eq!(w.fact(task).await, None, "a draft is not published");
        let columns: Vec<String> = sqlx::query_scalar!(
            r#"SELECT column_name AS "name!" FROM information_schema.columns
               WHERE table_schema = 'content' AND table_name = 'facts'"#
        )
        .fetch_all(&w.env.pool)
        .await
        .unwrap();
        assert!(
            columns.iter().all(|name| !name.contains("draft")),
            "{columns:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn a_draft_submitted_by_the_scheduler_leaves_the_same_fact_as_a_student_submit() {
    db_test(async |pool| {
        let w = world(pool).await;
        let due = Utc::now() + Duration::hours(1);
        let task = id_of(
            &w.env
                .ok(
                    "POST",
                    &w.teacher.token,
                    &format!("/modules/{}/items", w.module),
                    Some(json!({"kind": "assignment", "title": "A", "assignment": {
                        "type": "text", "max_points": "10",
                        "due_at": due.to_rfc3339(), "on_deadline": "auto_submit",
                        "accept_late": false}})),
                    201,
                )
                .await,
        );
        w.go(
            &w.teacher,
            "POST",
            &format!("/items/{task}/publish"),
            None,
            200,
        )
        .await;
        w.go(
            &w.student,
            "PUT",
            &format!("/assignments/{task}/submission/draft"),
            Some(json!({"content": essay("unfinished")})),
            200,
        )
        .await;

        tick(
            &w.state.db,
            &w.state.learning.editor,
            due + Duration::seconds(1),
        )
        .await
        .expect("a tick");

        let (submitted, _, _) = w
            .fact(task)
            .await
            .expect("the scheduler's submit left a fact");
        assert_eq!(submitted, Some(w.first_attempt_at(task).await));
    })
    .await;
}

#[tokio::test]
async fn a_rebuild_makes_the_rows_the_events_made() {
    db_test(async |pool| {
        let w = world(pool).await;
        let (first, second) = (w.task().await, w.task().await);
        let lecture = w.lecture().await;
        w.submit(first).await;
        w.grade(first, Some("7")).await;
        w.submit(second).await;
        w.grade(second, Some("5")).await;
        w.grade(second, None).await;
        w.mark(lecture).await;
        let made = w.snapshot().await;
        assert_eq!(made.len(), 3);

        let mut conn = w.env.pool.acquire().await.unwrap();
        replace_source_facts(&mut conn, &FactScope::All, &[])
            .await
            .unwrap();
        assert_ne!(w.snapshot().await, made, "the wipe took the source columns");
        let mut tx = w.env.pool.begin().await.unwrap();
        learning::rebuild_facts(&mut tx, None).await.unwrap();
        tx.commit().await.unwrap();

        assert_eq!(w.snapshot().await, made);
    })
    .await;
}

#[tokio::test]
async fn a_rebuild_of_a_course_keeps_the_marks() {
    db_test(async |pool| {
        let w = world(pool).await;
        let lecture = w.lecture().await;
        w.mark(lecture).await;
        let course =
            sqlx::query_scalar!("SELECT course_id FROM content.items WHERE id = $1", lecture)
                .fetch_one(&w.env.pool)
                .await
                .unwrap();
        let before = w.fact(lecture).await.unwrap();
        let mut tx = w.env.pool.begin().await.unwrap();

        learning::rebuild_facts(&mut tx, Some(course))
            .await
            .unwrap();
        tx.commit().await.unwrap();

        assert!(before.2, "the student marked it");
        assert_eq!(
            w.fact(lecture).await,
            Some(before),
            "the rebuild left the mark"
        );
    })
    .await;
}

#[tokio::test]
async fn two_transactions_for_one_pair_do_not_overwrite_each_other() {
    db_test(async |pool| {
        let w = world(pool).await;
        let task = w.task().await;
        w.submit(task).await;
        w.draft(task, Some("8")).await;
        let scope = FactScope::Pair {
            item_id: task,
            student_id: w.student.id,
        };

        // One transaction takes the pair and, still holding it, publishes the grade.
        let mut holder = w.env.pool.begin().await.unwrap();
        content::lock_facts(&mut holder, &scope).await.unwrap();
        sqlx::query!(
            "UPDATE gradebook.grades SET published_score = draft_score,
                    published_by = $3, published_at = now()
             WHERE assignment_id = $1 AND student_id = $2",
            task,
            w.student.id,
            w.teacher.id
        )
        .execute(&mut *holder)
        .await
        .unwrap();

        // A second one, for the same pair, has to wait for it, and then reads what it committed.
        let pool = w.env.pool.clone();
        let waiting = tokio::spawn(async move {
            let mut tx = pool.begin().await.unwrap();
            learning::facts::sync(&mut tx, scope).await.unwrap();
            tx.commit().await.unwrap();
        });
        test_support::wait_until_blocked(&w.env.pool, &waiting).await;
        holder.commit().await.unwrap();
        waiting.await.unwrap();

        let (submitted, score, _) = w.fact(task).await.unwrap();
        assert!(submitted.is_some());
        assert_eq!(score.as_deref(), Some("8.00"));
    })
    .await;
}

impl World {
    /// A published assignment of the given type.
    async fn made(&self, assignment: Value) -> Uuid {
        let item = id_of(
            &self
                .env
                .ok(
                    "POST",
                    &self.teacher.token,
                    &format!("/modules/{}/items", self.module),
                    Some(json!({"kind": "assignment", "title": "A", "assignment": assignment})),
                    201,
                )
                .await,
        );
        self.go(
            &self.teacher,
            "POST",
            &format!("/items/{item}/publish"),
            None,
            200,
        )
        .await;
        item
    }

    /// Writes the student's row for the item as the sources would have it, with nothing else.
    async fn row(&self, item: Uuid, submitted: bool, score: Option<&str>) {
        let mut conn = self.env.pool.acquire().await.unwrap();
        let fact = SourceFact {
            item_id: item,
            student_id: self.student.id,
            submitted_at: submitted.then(Utc::now),
            published_score: score.map(|score| score.parse().unwrap()),
        };
        replace_source_facts(&mut conn, &FactScope::All, &[fact])
            .await
            .unwrap();
    }

    /// Whether the course tells the student the item is done.
    async fn is_done(&self, item: Uuid) -> bool {
        let seen = self
            .env
            .ok(
                "GET",
                &self.student.token,
                &format!("/items/{item}"),
                None,
                200,
            )
            .await;
        seen["standing"]["done"] == json!(true)
    }
}

#[tokio::test]
async fn a_no_submission_assignment_is_done_only_when_its_score_is_published() {
    db_test(async |pool| {
        let w = world(pool).await;
        let oral = w
            .made(json!({"type": "no_submission", "max_points": "5"}))
            .await;
        assert!(!w.is_done(oral).await);

        // A row that holds a submit and no score is not a grade.
        w.row(oral, true, None).await;
        assert!(!w.is_done(oral).await);

        w.row(oral, true, Some("4")).await;
        assert!(w.is_done(oral).await);
    })
    .await;
}

#[tokio::test]
async fn a_quiz_is_done_at_submit_before_grading() {
    db_test(async |pool| {
        let w = world(pool).await;
        let option = |id: &str| json!({"id": id, "content": essay(id)});
        let quiz = w
            .made(json!({"type": "quiz", "max_points": "1", "config": {
                "v": 1, "shuffle_questions": false, "show_correct": false,
                "questions": [{"id": "q1", "type": "single_choice", "points": "1",
                    "prompt": essay("which"), "options": [option("a"), option("b")],
                    "correct": ["a"]}]}}))
            .await;
        assert!(!w.is_done(quiz).await);

        w.row(quiz, true, None).await;

        assert!(w.is_done(quiz).await);
    })
    .await;
}

#[tokio::test]
async fn a_text_assignment_is_done_at_submit_and_a_text_item_when_marked() {
    db_test(async |pool| {
        let w = world(pool).await;
        let task = w.task().await;
        let lecture = w.lecture().await;
        assert!(!w.is_done(task).await && !w.is_done(lecture).await);

        w.submit(task).await;
        assert!(w.is_done(task).await);
        assert!(!w.is_done(lecture).await);

        w.mark(lecture).await;
        assert!(w.is_done(lecture).await);
    })
    .await;
}

#[tokio::test]
async fn a_source_fact_of_an_item_that_is_gone_is_skipped() {
    db_test(async |pool| {
        let w = world(pool).await;
        let mut conn = w.env.pool.acquire().await.unwrap();
        let ghost = SourceFact {
            item_id: Uuid::now_v7(),
            student_id: w.student.id,
            submitted_at: Some(Utc::now()),
            published_score: None,
        };

        replace_source_facts(&mut conn, &FactScope::All, &[ghost])
            .await
            .unwrap();

        assert!(w.snapshot().await.is_empty());
    })
    .await;
}
