//! Lock order (ADR-0028 point 5, READ COMMITTED). Each test holds one writer's transaction
//! open, starts a second writer on a task, waits until that writer is observed blocked on a
//! lock, and only then lets the first one commit. Nothing sleeps: the order is forced by the
//! locks, not by timing.
use crate::test_support::{db_test, wait_until_blocked};

use std::sync::LazyLock;

use serde_json::Map;
use sqlx::PgPool;

use super::*;
use crate::content::{
    self, Assignment, BuiltinType, ContentError, ContentTitle, EffectiveProfiles, ItemPatch,
    ItemRules, MaxPoints, SubmissionSettings,
};
use crate::identity::{self, Email, NewUser, PersonName};
use crate::policy::CourseRole;
use crate::teaching::{self, CourseTitle};
use common::PageParams;
use common::Role;
use vumlo_config::GradingScale;

static PROFILES: LazyLock<EffectiveProfiles> = LazyLock::new(EffectiveProfiles::unrestricted);

fn rules() -> ItemRules<'static> {
    ItemRules {
        scale: GradingScale::Hundred,
        profiles: &PROFILES,
    }
}

struct Fixture {
    pool: PgPool,
    owner: Actor,
    course: Uuid,
    student: Uuid,
    assignment: Uuid,
}

/// A course with a teacher, a student and one assignment worth 10 points.
async fn fixture(pool: PgPool) -> Fixture {
    fixture_worth(pool, "10").await
}

/// The same, with an assignment worth `points`.
async fn fixture_worth(pool: PgPool, points: &str) -> Fixture {
    let hash = identity::hash_password("irrelevant").await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    let mut create = async |email: &str, role: Role| {
        identity::create_user_unaudited_for_tests(
            &mut tx,
            NewUser {
                email: Email::parse(email).unwrap(),
                name: PersonName::name("Test").unwrap(),
                surname: PersonName::surname("User").unwrap(),
                role,
            },
            hash.clone(),
        )
        .await
        .unwrap()
    };
    let owner_user = create("owner@x.edu", Role::Teacher).await;
    let student_user = create("student@x.edu", Role::Student).await;
    let owner = Actor::new(owner_user.id, Role::Teacher, Uuid::now_v7());
    let course = teaching::create_course(
        &mut tx,
        &owner,
        CourseTitle::parse("Algebra").unwrap(),
        None,
    )
    .await
    .unwrap();
    teaching::put_member(
        &mut tx,
        &owner,
        course.id,
        student_user.id,
        CourseRole::Student,
    )
    .await
    .unwrap();
    let module = content::create_module(
        &mut tx,
        &owner,
        course.id,
        ContentTitle::parse("M").unwrap(),
        None,
    )
    .await
    .unwrap();
    let item = content::create_item(
        &mut tx,
        &owner,
        rules(),
        module.id,
        ContentTitle::parse("A").unwrap(),
        None,
        Some(Assignment {
            assignment_type: BuiltinType::Text,
            config: Map::new(),
            max_points: MaxPoints::parse(points).unwrap(),
            is_bonus: false,
            settings: SubmissionSettings::default(),
            class_on: None,
        }),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    Fixture {
        pool,
        owner,
        course: course.id,
        student: student_user.id,
        assignment: item.id,
    }
}

fn score(text: &str) -> GradeInput {
    GradeInput {
        score: Some(text.to_owned()),
        feedback: None,
    }
}

fn lower_to(points: &str) -> ItemPatch {
    ItemPatch {
        max_points: Some(MaxPoints::parse(points).unwrap()),
        ..ItemPatch::default()
    }
}

impl Fixture {
    /// Saves a grade in its own transaction on a task and commits when it succeeds.
    fn save_in_background(
        &self,
        text: &'static str,
    ) -> tokio::task::JoinHandle<Result<Cell, GradebookError>> {
        let (pool, owner, student, assignment) = (
            self.pool.clone(),
            self.owner.clone(),
            self.student,
            self.assignment,
        );
        tokio::spawn(async move {
            let mut tx = pool.begin().await.unwrap();
            let result = save_grade(&mut tx, &owner, assignment, student, score(text)).await;
            if result.is_ok() {
                tx.commit().await.unwrap();
            }
            result
        })
    }

    /// Lowers `max_points` in its own transaction on a task and commits when it succeeds.
    fn lower_in_background(
        &self,
        points: &'static str,
    ) -> tokio::task::JoinHandle<Result<(), ContentError>> {
        let (pool, owner, assignment) = (self.pool.clone(), self.owner.clone(), self.assignment);
        tokio::spawn(async move {
            let mut tx = pool.begin().await.unwrap();
            let result =
                content::update_item(&mut tx, &owner, rules(), assignment, lower_to(points))
                    .await
                    .map(|_| ());
            if result.is_ok() {
                tx.commit().await.unwrap();
            }
            result
        })
    }

    /// Deletes the assignment in its own transaction on a task and commits when it succeeds.
    fn delete_in_background(&self) -> tokio::task::JoinHandle<Result<(), ContentError>> {
        let (pool, owner, assignment) = (self.pool.clone(), self.owner.clone(), self.assignment);
        tokio::spawn(async move {
            let mut tx = pool.begin().await.unwrap();
            let result = content::delete_item(&mut tx, &owner, assignment).await;
            if result.is_ok() {
                tx.commit().await.unwrap();
            }
            result
        })
    }

    /// Publishes every student's draft in its own transaction on a task and commits when it
    /// succeeds.
    fn publish_in_background(&self) -> tokio::task::JoinHandle<Result<Vec<Uuid>, GradebookError>> {
        let (pool, owner, assignment) = (self.pool.clone(), self.owner.clone(), self.assignment);
        tokio::spawn(async move {
            let mut tx = pool.begin().await.unwrap();
            let result = publish_grades(&mut tx, &owner, assignment, Students::All).await;
            if result.is_ok() {
                tx.commit().await.unwrap();
            }
            result
        })
    }

    /// Saves a draft score in a transaction that is committed at once.
    async fn save_committed(&self, text: &'static str) {
        let mut tx = self.pool.begin().await.unwrap();
        save_grade(
            &mut tx,
            &self.owner,
            self.assignment,
            self.student,
            score(text),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }

    /// The cell as stored: draft score, published score, and whether it was never published.
    async fn stored(&self) -> (Option<String>, Option<String>, bool) {
        let row = sqlx::query!(
            r#"SELECT draft_score::text AS draft, published_score::text AS published,
                          published_at IS NULL AS "never_published!"
                   FROM gradebook.grades WHERE assignment_id = $1"#,
            self.assignment
        )
        .fetch_one(&self.pool)
        .await
        .unwrap();
        (row.draft, row.published, row.never_published)
    }

    /// The history of the cell, oldest first: the action and the score after it.
    async fn history(&self) -> Vec<(String, Option<String>)> {
        sqlx::query!(
            "SELECT action, score::text AS score FROM gradebook.grade_history
                 WHERE assignment_id = $1 ORDER BY at, id",
            self.assignment
        )
        .fetch_all(&self.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.action, row.score))
        .collect()
    }

    async fn published_events(&self) -> i64 {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM events.outbox WHERE type = 'grade.published'"#
        )
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    /// What the staff gradebook says about the student's only cell.
    async fn has_unpublished_changes(&self) -> bool {
        let mut conn = self.pool.acquire().await.unwrap();
        let page = gradebook(
            &mut conn,
            &self.owner,
            self.course,
            PageParams::new(None, None).unwrap(),
        )
        .await
        .unwrap();
        page.rows[0].cells[0].1.has_unpublished_changes
    }

    async fn grades(&self) -> i64 {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM gradebook.grades WHERE assignment_id = $1"#,
            self.assignment
        )
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn assignment_rows(&self) -> i64 {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM content.assignments WHERE item_id = $1"#,
            self.assignment
        )
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn item_rows(&self) -> i64 {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM content.items WHERE id = $1"#,
            self.assignment
        )
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn max_points(&self) -> String {
        sqlx::query_scalar!(
                r#"SELECT max_points::text AS "max_points!" FROM content.assignments WHERE item_id = $1"#,
                self.assignment
            )
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
}

/// A grade write is in flight with an uncommitted score of 8. A lowering to 5 must wait for
/// it and then see the score. Without the row locks it would not see it and lower the points
/// under a score of 8.
#[tokio::test]
async fn a_lowering_waits_for_a_grade_write_and_then_sees_its_score() {
    db_test(async |pool| {
        let f = fixture(pool).await;

        let mut first = f.pool.begin().await.unwrap();
        save_grade(&mut first, &f.owner, f.assignment, f.student, score("8"))
            .await
            .unwrap();
        let second = f.lower_in_background("5");
        wait_until_blocked(&f.pool, &second).await;
        first.commit().await.unwrap();

        let result = second.await.unwrap();
        assert!(
            matches!(result, Err(ContentError::BelowGivenScores)),
            "{result:?}"
        );
        assert_eq!(f.max_points().await, "10.00");
        assert_eq!(f.grades().await, 1);
    })
    .await;
}

/// A lowering to 5 is in flight and holds the row. A grade write of 8, valid against the old
/// maximum, must wait for it and be checked against the new one. Without the lock the write
/// would read 10, store 8 and leave a score above `max_points`.
#[tokio::test]
async fn a_grade_write_waits_for_a_lowering_and_is_checked_against_the_new_maximum() {
    db_test(async |pool| {
        let f = fixture(pool).await;

        let mut first = f.pool.begin().await.unwrap();
        content::update_item(&mut first, &f.owner, rules(), f.assignment, lower_to("5"))
            .await
            .unwrap();
        let second = f.save_in_background("8");
        wait_until_blocked(&f.pool, &second).await;
        first.commit().await.unwrap();

        let result = second.await.unwrap();
        match result {
            Err(GradebookError::Invalid(problems)) => {
                assert_eq!(problems.len(), 1);
                assert_eq!(problems[0].field, "score");
                assert!(
                    problems[0].message.contains("5.00"),
                    "{}",
                    problems[0].message
                );
            }
            other => panic!("expected the score to be refused, got {other:?}"),
        }
        assert_eq!(f.max_points().await, "5.00");
        assert_eq!(f.grades().await, 0);
    })
    .await;
}

/// A grade write is in flight. A delete of the assignment must wait for it and then see the
/// grade. Without the lock the delete would pass its check, remove the assignment and leave
/// the grade behind with nothing to belong to.
#[tokio::test]
async fn a_delete_waits_for_a_grade_write_and_then_sees_the_grade() {
    db_test(async |pool| {
        let f = fixture(pool).await;

        let mut first = f.pool.begin().await.unwrap();
        save_grade(&mut first, &f.owner, f.assignment, f.student, score("8"))
            .await
            .unwrap();
        let second = f.delete_in_background();
        wait_until_blocked(&f.pool, &second).await;
        first.commit().await.unwrap();

        let result = second.await.unwrap();
        assert!(matches!(result, Err(ContentError::HasGrades)), "{result:?}");
        assert_eq!(f.grades().await, 1);
        assert_eq!(f.assignment_rows().await, 1);
    })
    .await;
}

/// A delete of the assignment is in flight. A grade write must wait for it and then find
/// that the assignment is gone, instead of storing a grade for it.
#[tokio::test]
async fn a_grade_write_waits_for_a_delete_and_then_finds_the_assignment_gone() {
    db_test(async |pool| {
        let f = fixture(pool).await;

        let mut first = f.pool.begin().await.unwrap();
        content::delete_item(&mut first, &f.owner, f.assignment)
            .await
            .unwrap();
        let second = f.save_in_background("8");
        wait_until_blocked(&f.pool, &second).await;
        first.commit().await.unwrap();

        let result = second.await.unwrap();
        assert!(
            matches!(result, Err(GradebookError::AssignmentNotFound)),
            "{result:?}"
        );
        assert_eq!(f.grades().await, 0);
        assert_eq!(f.item_rows().await, 0);
    })
    .await;
}

fn history_of(rows: &[(&str, Option<&str>)]) -> Vec<(String, Option<String>)> {
    rows.iter()
        .map(|(action, score)| ((*action).to_owned(), score.map(str::to_owned)))
        .collect()
}

/// The draft is saved (10) and committed. Another save (15) is in flight and holds the cell
/// when a publish starts. The publish must wait for it and copy the draft as it is after the
/// wait: 15, not the 10 it could have read before. Without that, the student would be shown
/// a grade that no longer is the teacher's draft, while the history and the event say 15.
#[tokio::test]
async fn a_publish_waits_for_a_draft_save_and_copies_the_draft_as_it_is_after_the_wait() {
    db_test(async |pool| {
        let f = fixture_worth(pool, "20").await;
        f.save_committed("10").await;

        let mut holder = f.pool.begin().await.unwrap();
        save_grade(&mut holder, &f.owner, f.assignment, f.student, score("15"))
            .await
            .unwrap();
        let publish = f.publish_in_background();
        wait_until_blocked(&f.pool, &publish).await;
        holder.commit().await.unwrap();

        assert_eq!(publish.await.unwrap().unwrap(), [f.student]);
        let (draft, published, never_published) = f.stored().await;
        assert_eq!(draft.as_deref(), Some("15.00"));
        assert_eq!(published.as_deref(), Some("15.00"));
        assert!(!never_published);
        assert_eq!(
            f.history().await,
            history_of(&[
                ("save_draft", Some("10.00")),
                ("save_draft", Some("15.00")),
                ("publish", Some("15.00")),
            ])
        );
        assert_eq!(f.published_events().await, 1);
    })
    .await;
}

/// Same start, but the save in flight clears the draft. After the wait there is nothing to
/// publish for that row: no student is returned, nothing is published, nothing is logged as a
/// publish and no event is emitted.
#[tokio::test]
async fn a_publish_waits_for_a_clearing_save_and_then_has_nothing_to_publish() {
    db_test(async |pool| {
        let f = fixture(pool).await;
        f.save_committed("10").await;

        let mut holder = f.pool.begin().await.unwrap();
        save_grade(
            &mut holder,
            &f.owner,
            f.assignment,
            f.student,
            GradeInput {
                score: None,
                feedback: None,
            },
        )
        .await
        .unwrap();
        let publish = f.publish_in_background();
        wait_until_blocked(&f.pool, &publish).await;
        holder.commit().await.unwrap();

        assert!(publish.await.unwrap().unwrap().is_empty());
        let (draft, published, never_published) = f.stored().await;
        assert_eq!((draft, published), (None, None));
        assert!(never_published, "the row was not published");
        assert_eq!(
            f.history().await,
            history_of(&[("save_draft", Some("10.00")), ("save_draft", None)])
        );
        assert_eq!(f.published_events().await, 0);
    })
    .await;
}

/// The opposite order: a publish has copied the draft (10) and holds the cell. A save of 15
/// starts and must wait for it. Once the publish commits the save lands on top: the student
/// keeps the 10 that was published, the draft is 15 and the cell has unpublished changes.
#[tokio::test]
async fn a_draft_save_waits_for_a_publish_and_lands_on_top_of_it() {
    db_test(async |pool| {
        let f = fixture_worth(pool, "20").await;
        f.save_committed("10").await;

        let mut holder = f.pool.begin().await.unwrap();
        let published = publish_grades(&mut holder, &f.owner, f.assignment, Students::All)
            .await
            .unwrap();
        assert_eq!(published, [f.student]);
        let save = f.save_in_background("15");
        wait_until_blocked(&f.pool, &save).await;
        holder.commit().await.unwrap();

        save.await.unwrap().unwrap();
        let (draft, published, _) = f.stored().await;
        assert_eq!(draft.as_deref(), Some("15.00"));
        assert_eq!(published.as_deref(), Some("10.00"));
        assert!(f.has_unpublished_changes().await);
        assert_eq!(
            f.history().await,
            history_of(&[
                ("save_draft", Some("10.00")),
                ("publish", Some("10.00")),
                ("save_draft", Some("15.00")),
            ])
        );
        assert_eq!(f.published_events().await, 1);
    })
    .await;
}
