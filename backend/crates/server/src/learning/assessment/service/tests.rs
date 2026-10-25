//! The clock comes from the caller, so the boundaries of the window are tested with explicit
//! moments. Lock order (ADR-0035 point 12, READ COMMITTED): each concurrency test holds one
//! transaction open, starts a second on a task, waits until that task is observed blocked on
//! a lock, and only then lets the first one commit. Nothing sleeps: the order is forced by
//! the locks, not by timing.
use test_support::{db_test, wait_until_blocked};

use std::sync::LazyLock;

use serde_json::{Map, json};
use sqlx::PgPool;

use super::*;
use crate::auth::{self, Email, NewUser, PersonName};
use crate::learning::content::{
    Assignment, BuiltinType, ContentError, ContentTitle, ItemPatch, ItemRules, MaxPoints,
    OnDeadline, SettingsPatch, SubmissionSettings,
};
use crate::learning::teaching::{self, CourseTitle};
use access::CourseRole;
use common::Role;
use vumlo_config::GradingScale;

static PROFILES: LazyLock<EffectiveProfiles> = LazyLock::new(EffectiveProfiles::unrestricted);

fn rules() -> ItemRules<'static> {
    ItemRules {
        scale: GradingScale::Hundred,
        profiles: &PROFILES,
    }
}

fn at(hour: u32, minute: u32) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&format!("2030-05-01T{hour:02}:{minute:02}:00Z"))
        .unwrap()
        .with_timezone(&Utc)
}

fn essay(text: &str) -> Value {
    json!({"v": 1, "attachments": [], "blocks": [
        {"id": "p", "type": "paragraph", "content": [{"type": "text", "text": text}]}
    ]})
}

struct Fixture {
    pool: PgPool,
    owner: Actor,
    student: Actor,
    course: Uuid,
    assignment: Uuid,
}

/// A published course with a teacher, a student and one published assignment with `settings`.
async fn fixture(pool: PgPool, settings: SubmissionSettings) -> Fixture {
    let hash = auth::hash_password("irrelevant").await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    let mut create = async |email: &str, role: Role| {
        auth::create_user_unaudited_for_tests(
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
    let owner = Actor::for_tests(owner_user.id, Role::Teacher, Uuid::now_v7());
    let student = Actor::for_tests(student_user.id, Role::Student, Uuid::now_v7());
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
    teaching::publish_course(&mut tx, &owner, course.id)
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
    content::publish_module(&mut tx, &owner, module.id, true)
        .await
        .unwrap();
    let item = content::create_item(
        &mut tx,
        &owner,
        rules(),
        module.id,
        ContentTitle::parse("Essay").unwrap(),
        None,
        Some(Assignment {
            assignment_type: BuiltinType::Text,
            config: Map::new(),
            max_points: MaxPoints::parse("10").unwrap(),
            is_bonus: false,
            settings,
            class_on: None,
        }),
    )
    .await
    .unwrap();
    content::publish_item(&mut tx, &owner, item.id)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    Fixture {
        pool,
        owner,
        student,
        course: course.id,
        assignment: item.id,
    }
}

fn due_at_ten(accept_late: bool, late_until: Option<DateTime<Utc>>) -> SubmissionSettings {
    SubmissionSettings {
        due_at: Some(at(10, 0)),
        on_deadline: Some(OnDeadline::KeepDraft),
        accept_late: Some(accept_late),
        late_until,
        max_attempts: None,
    }
}

impl Fixture {
    async fn save(&self, text: &str, now: DateTime<Utc>) -> Result<Draft, AssessmentError> {
        let mut tx = self.pool.begin().await.unwrap();
        let result = save_draft(
            &mut tx,
            &self.student,
            self.assignment,
            &essay(text),
            &PROFILES,
            now,
        )
        .await;
        if result.is_ok() {
            tx.commit().await.unwrap();
        }
        result
    }

    async fn submit(&self, now: DateTime<Utc>) -> Result<Attempt, AssessmentError> {
        let mut tx = self.pool.begin().await.unwrap();
        let result = submit(&mut tx, &self.student, self.assignment, &PROFILES, now).await;
        if result.is_ok() {
            tx.commit().await.unwrap();
        }
        result
    }

    /// Submits in its own transaction on a task and commits when it succeeds.
    fn submit_in_background(
        &self,
        now: DateTime<Utc>,
    ) -> tokio::task::JoinHandle<Result<Attempt, AssessmentError>> {
        let (pool, student, assignment) =
            (self.pool.clone(), self.student.clone(), self.assignment);
        tokio::spawn(async move {
            let mut tx = pool.begin().await.unwrap();
            let result = submit(&mut tx, &student, assignment, &PROFILES, now).await;
            if result.is_ok() {
                tx.commit().await.unwrap();
            }
            result
        })
    }

    fn save_in_background(
        &self,
        text: &'static str,
        now: DateTime<Utc>,
    ) -> tokio::task::JoinHandle<Result<Draft, AssessmentError>> {
        let (pool, student, assignment) =
            (self.pool.clone(), self.student.clone(), self.assignment);
        tokio::spawn(async move {
            let mut tx = pool.begin().await.unwrap();
            let result =
                save_draft(&mut tx, &student, assignment, &essay(text), &PROFILES, now).await;
            if result.is_ok() {
                tx.commit().await.unwrap();
            }
            result
        })
    }

    /// Changes the settings in its own transaction on a task and commits when it succeeds.
    fn settings_in_background(
        &self,
        patch: SettingsPatch,
    ) -> tokio::task::JoinHandle<Result<(), ContentError>> {
        let (pool, owner, assignment) = (self.pool.clone(), self.owner.clone(), self.assignment);
        tokio::spawn(async move {
            let mut tx = pool.begin().await.unwrap();
            let patch = ItemPatch {
                settings: patch,
                ..ItemPatch::default()
            };
            let result = content::update_item(&mut tx, &owner, rules(), assignment, patch)
                .await
                .map(|_| ());
            if result.is_ok() {
                tx.commit().await.unwrap();
            }
            result
        })
    }

    async fn rows(&self, status: &str) -> i64 {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM assessment.submissions
                   WHERE assignment_id = $1 AND status = $2"#,
            self.assignment,
            status
        )
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn events(&self) -> i64 {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM events.outbox WHERE type = 'submission.submitted'"#
        )
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }
}

// --- the clock is a parameter ----------------------------------------------------------

#[tokio::test]
async fn work_at_the_deadline_is_on_time_and_a_moment_later_it_is_not() {
    db_test(async |pool| {
        let f = fixture(pool, due_at_ten(false, None)).await;
        f.save("on the dot", at(10, 0)).await.unwrap();
        let attempt = f.submit(at(10, 0)).await.unwrap();
        assert!(!attempt.is_late);
        assert_eq!(
            attempt.submitted_at,
            at(10, 0),
            "the stored time is the caller's"
        );

        let later = at(10, 0) + chrono::Duration::milliseconds(1);
        assert!(matches!(
            f.save("late", later).await,
            Err(AssessmentError::WindowClosed)
        ));
        assert!(matches!(
            f.submit(later).await,
            Err(AssessmentError::WindowClosed)
        ));
    })
    .await;
}

#[tokio::test]
async fn late_work_is_marked_until_late_until_inclusive() {
    db_test(async |pool| {
        let f = fixture(pool, due_at_ten(true, Some(at(12, 0)))).await;
        f.save("one", at(9, 0)).await.unwrap();
        assert!(!f.submit(at(9, 0)).await.unwrap().is_late);
        f.save("two", at(11, 0)).await.unwrap();
        assert!(f.submit(at(11, 0)).await.unwrap().is_late);
        f.save("three", at(12, 0)).await.unwrap();
        assert!(
            f.submit(at(12, 0)).await.unwrap().is_late,
            "late_until itself is inside"
        );

        let after = at(12, 0) + chrono::Duration::milliseconds(1);
        assert!(matches!(
            f.save("four", after).await,
            Err(AssessmentError::WindowClosed)
        ));
        assert_eq!(f.rows("submitted").await, 3);
        assert_eq!(f.rows("draft").await, 0);
    })
    .await;
}

// --- locks -----------------------------------------------------------------------------

/// A settings change is in flight and holds the assignment row. A submit that began after it
/// must wait, and then be judged by the new window: the deadline has passed, so it is refused
/// and the draft stays.
#[tokio::test]
async fn a_submit_waits_for_a_settings_change_and_sees_the_new_window() {
    db_test(async |pool| {
        let f = fixture(pool, due_at_ten(false, None)).await;
        f.save("ready", at(9, 0)).await.unwrap();
        let now = at(9, 30);

        let mut holder = f.pool.begin().await.unwrap();
        let patch = ItemPatch {
            settings: SettingsPatch {
                due_at: Some(Some(at(9, 0))),
                ..SettingsPatch::default()
            },
            ..ItemPatch::default()
        };
        content::update_item(&mut holder, &f.owner, rules(), f.assignment, patch)
            .await
            .unwrap();
        let submit = f.submit_in_background(now);
        wait_until_blocked(&f.pool, &submit).await;
        holder.commit().await.unwrap();

        let result = submit.await.unwrap();
        assert!(
            matches!(result, Err(AssessmentError::WindowClosed)),
            "{result:?}"
        );
        assert_eq!(f.rows("submitted").await, 0);
        assert_eq!(f.rows("draft").await, 1, "the draft stays");
        assert_eq!(f.events().await, 0);
    })
    .await;
}

/// The opposite order: a submit is in flight and holds the row `FOR SHARE`. A settings change
/// waits for it. The submit was judged by the old window and stays on time; the change
/// applies afterwards.
#[tokio::test]
async fn a_settings_change_waits_for_a_submit_in_flight_and_applies_after_it() {
    db_test(async |pool| {
        let f = fixture(pool, due_at_ten(false, None)).await;
        f.save("ready", at(9, 0)).await.unwrap();

        let mut holder = f.pool.begin().await.unwrap();
        let attempt = submit(&mut holder, &f.student, f.assignment, &PROFILES, at(9, 30))
            .await
            .unwrap();
        assert!(!attempt.is_late);
        let change = f.settings_in_background(SettingsPatch {
            due_at: Some(Some(at(9, 0))),
            ..SettingsPatch::default()
        });
        wait_until_blocked(&f.pool, &change).await;
        holder.commit().await.unwrap();

        change.await.unwrap().unwrap();
        assert_eq!(f.rows("submitted").await, 1, "the submit stands");
        let due = sqlx::query_scalar!(
            "SELECT due_at FROM content.assignments WHERE item_id = $1",
            f.assignment
        )
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert_eq!(due, Some(at(9, 0)));
    })
    .await;
}

/// Two submits of one draft: the second waits for the draft row and, once the first commits,
/// finds no draft. Exactly one attempt and one event exist.
#[tokio::test]
async fn of_two_submits_of_one_draft_exactly_one_wins() {
    db_test(async |pool| {
        let f = fixture(pool, SubmissionSettings::default()).await;
        f.save("once", at(9, 0)).await.unwrap();

        let mut first = f.pool.begin().await.unwrap();
        let attempt = submit(&mut first, &f.student, f.assignment, &PROFILES, at(9, 1))
            .await
            .unwrap();
        assert_eq!(attempt.attempt, 1);
        let second = f.submit_in_background(at(9, 2));
        wait_until_blocked(&f.pool, &second).await;
        first.commit().await.unwrap();

        let result = second.await.unwrap();
        assert!(
            matches!(result, Err(AssessmentError::NoDraft)),
            "{result:?}"
        );
        assert_eq!(f.rows("submitted").await, 1);
        assert_eq!(f.rows("draft").await, 0);
        assert_eq!(f.events().await, 1);
    })
    .await;
}

/// A draft save that begins while a submit holds the draft row waits for it and then starts
/// a new draft: the submitted attempt is not overwritten.
#[tokio::test]
async fn a_draft_save_waits_for_a_submit_and_then_starts_a_new_draft() {
    db_test(async |pool| {
        let f = fixture(pool, SubmissionSettings::default()).await;
        f.save("first", at(9, 0)).await.unwrap();

        let mut holder = f.pool.begin().await.unwrap();
        submit(&mut holder, &f.student, f.assignment, &PROFILES, at(9, 1))
            .await
            .unwrap();
        let save = f.save_in_background("second", at(9, 2));
        wait_until_blocked(&f.pool, &save).await;
        holder.commit().await.unwrap();

        save.await.unwrap().unwrap();
        assert_eq!(f.rows("submitted").await, 1);
        assert_eq!(f.rows("draft").await, 1);
        let kept = sqlx::query_scalar!(
            r#"SELECT content AS "content: serde_json::Value" FROM assessment.submissions
                   WHERE assignment_id = $1 AND status = 'submitted'"#,
            f.assignment
        )
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert_eq!(kept, essay("first"), "the attempt keeps what was submitted");
    })
    .await;
}

/// Deletes the assignment in its own transaction on a task and commits when it succeeds.
fn delete_in_background(f: &Fixture) -> tokio::task::JoinHandle<Result<(), ContentError>> {
    let (pool, owner, assignment) = (f.pool.clone(), f.owner.clone(), f.assignment);
    tokio::spawn(async move {
        let mut tx = pool.begin().await.unwrap();
        let result = content::delete_item(&mut tx, &owner, assignment).await;
        if result.is_ok() {
            tx.commit().await.unwrap();
        }
        result
    })
}

async fn assignment_rows(f: &Fixture) -> i64 {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM content.assignments WHERE item_id = $1"#,
        f.assignment
    )
    .fetch_one(&f.pool)
    .await
    .unwrap()
}

/// A submit is in flight and holds the assignment row `FOR SHARE`. A delete of the assignment
/// must wait for it and then see the attempt. Without the lock the delete would pass its
/// check, remove the assignment and leave the attempt with nothing to belong to.
#[tokio::test]
async fn a_delete_waits_for_a_submit_and_then_sees_the_attempt() {
    db_test(async |pool| {
        let f = fixture(pool, SubmissionSettings::default()).await;
        f.save("ready", at(9, 0)).await.unwrap();

        let mut first = f.pool.begin().await.unwrap();
        submit(&mut first, &f.student, f.assignment, &PROFILES, at(9, 1))
            .await
            .unwrap();
        let delete = delete_in_background(&f);
        wait_until_blocked(&f.pool, &delete).await;
        first.commit().await.unwrap();

        let result = delete.await.unwrap();
        assert!(
            matches!(result, Err(ContentError::HasSubmissions)),
            "{result:?}"
        );
        assert_eq!(f.rows("submitted").await, 1);
        assert_eq!(assignment_rows(&f).await, 1);
    })
    .await;
}

/// A delete is in flight and holds the row `FOR UPDATE`; the assignment had only a draft. A
/// submit must wait for it and then find the assignment gone, instead of storing an attempt
/// for it. The delete purged the draft with it.
#[tokio::test]
async fn a_submit_waits_for_a_delete_and_then_finds_the_assignment_gone() {
    db_test(async |pool| {
        let f = fixture(pool, SubmissionSettings::default()).await;
        f.save("ready", at(9, 0)).await.unwrap();

        let mut first = f.pool.begin().await.unwrap();
        content::delete_item(&mut first, &f.owner, f.assignment)
            .await
            .unwrap();
        let second = f.submit_in_background(at(9, 1));
        wait_until_blocked(&f.pool, &second).await;
        first.commit().await.unwrap();

        let result = second.await.unwrap();
        assert!(
            matches!(result, Err(AssessmentError::AssignmentNotFound)),
            "{result:?}"
        );
        assert_eq!(f.rows("submitted").await, 0);
        assert_eq!(
            f.rows("draft").await,
            0,
            "the draft went with the assignment"
        );
        assert_eq!(assignment_rows(&f).await, 0);
        assert_eq!(f.events().await, 0);
    })
    .await;
}

/// The same for a draft save: it waits for the delete and stores nothing for a gone
/// assignment.
#[tokio::test]
async fn a_draft_save_waits_for_a_delete_and_stores_nothing() {
    db_test(async |pool| {
        let f = fixture(pool, SubmissionSettings::default()).await;

        let mut first = f.pool.begin().await.unwrap();
        content::delete_item(&mut first, &f.owner, f.assignment)
            .await
            .unwrap();
        let save = f.save_in_background("late", at(9, 1));
        wait_until_blocked(&f.pool, &save).await;
        first.commit().await.unwrap();

        let result = save.await.unwrap();
        assert!(
            matches!(result, Err(AssessmentError::AssignmentNotFound)),
            "{result:?}"
        );
        assert_eq!(f.rows("draft").await, 0);
    })
    .await;
}

#[tokio::test]
async fn the_staff_reads_leave_a_student_with_only_a_draft_without_an_attempt() {
    db_test(async |pool| {
        let f = fixture(pool, SubmissionSettings::default()).await;
        f.save("DRAFT", at(9, 0)).await.unwrap();
        let mut conn = f.pool.acquire().await.unwrap();
        let page = PageParams::new(None, None).unwrap();
        let listed = list_submissions(&mut conn, &f.owner, f.assignment, page)
            .await
            .unwrap();
        assert_eq!(listed.items.len(), 1);
        assert_eq!(listed.items[0].latest, None);
        let attempts =
            student_attempts(&mut conn, &f.owner, f.assignment, f.student.user_id(), page)
                .await
                .unwrap();
        assert!(attempts.items.is_empty());
        assert!(
            !has_submitted_in_course(&mut conn, f.course).await.unwrap(),
            "a draft is not submitted work"
        );
    })
    .await;
}
