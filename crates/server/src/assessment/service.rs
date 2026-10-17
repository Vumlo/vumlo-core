//! Business rules. Every function that acts for a user authorizes first, takes the connection it
//! runs on and never begins or commits a transaction.
//!
//! An assignment the actor may not see does not exist as far as they can tell (404): an unknown
//! id, an id in a course they cannot see and a draft they may not open give the same error. Only
//! after that does the action decide between going ahead and 403.
//!
//! The time is a parameter of every function that depends on it: the handler reads the clock once.
//! `assessment` calls no other module's Rust code except the read-only functions that every module
//! uses to open and lock an assignment (ADR-0030, ADR-0035).

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{Attempt, Draft, MySubmission, StudentSubmission, Window};
use super::repo;
use crate::actor::Actor;
use crate::content::{
    self, Assignment, AssignmentType, DocumentErrors, EffectiveProfiles, OpenError,
    OpenedAssignment,
};
use crate::events::{self, Event, EventsError};
use crate::pagination::{Page, PageParams};
use crate::policy::{self, Action, PolicyError};
use crate::teaching::TeachingError;

#[derive(Debug, thiserror::Error)]
pub enum AssessmentError {
    #[error(transparent)]
    Forbidden(#[from] PolicyError),
    /// No such assignment, or the actor may not see it.
    #[error("assignment not found")]
    AssignmentNotFound,
    /// The user is neither a current student of the course nor the author of an attempt.
    #[error("student not found")]
    StudentNotFound,
    /// The assignment's type takes no submissions (`no_submission`).
    #[error("this assignment takes no submissions")]
    TakesNoSubmissions,
    /// The window closed at `due_at` or `late_until` (ADR-0035 point 5).
    #[error("the submission window is closed")]
    WindowClosed,
    /// A submit without a draft: a second submit, or nothing was saved.
    #[error("there is no draft to submit")]
    NoDraft,
    /// The student has used all their attempts.
    #[error("the attempt limit is reached")]
    AttemptLimit,
    /// Another submit of the same student got the attempt number first.
    #[error("a concurrent submit took this attempt")]
    Concurrent,
    /// The content is not a valid document of the student profile as the assignment narrows it.
    #[error("the content is not a valid document")]
    Document(DocumentErrors),
    /// A submit of a document with no text.
    #[error("the content is empty")]
    Empty,
    /// A purge found submitted attempts: the caller had to refuse the delete instead.
    #[error("there are submitted attempts")]
    HasSubmissions,
    #[error(transparent)]
    Teaching(TeachingError),
    #[error(transparent)]
    Events(#[from] EventsError),
    #[error(transparent)]
    Db(sqlx::Error),
}

impl From<sqlx::Error> for AssessmentError {
    /// The unique index on the attempt number is the backstop of the submit lock.
    fn from(error: sqlx::Error) -> Self {
        match error.as_database_error().and_then(|e| e.code()).as_deref() {
            Some("23505") => Self::Concurrent,
            _ => Self::Db(error),
        }
    }
}

/// Loads an assignment the actor may view, with their access to its course.
async fn open(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
) -> Result<OpenedAssignment, AssessmentError> {
    content::open_assignment(conn, actor, assignment_id)
        .await
        .map_err(|error| match error {
            OpenError::NotFound => AssessmentError::AssignmentNotFound,
            OpenError::Teaching(error) => AssessmentError::Teaching(error),
            OpenError::Db(error) => AssessmentError::Db(error),
        })
}

/// A type that takes no submissions has nothing to save, read or list. The type cannot change, so
/// the answer from the unlocked read is safe.
fn require_submissions(opened: &OpenedAssignment) -> Result<(), AssessmentError> {
    if opened.assignment_type.takes_submissions() {
        Ok(())
    } else {
        Err(AssessmentError::TakesNoSubmissions)
    }
}

/// Takes the assignment row `FOR SHARE` and returns it as it is in the row, with its settings and
/// `config`. A change of the settings and a delete take the row `FOR UPDATE`, so they wait for
/// this transaction, and what they committed before is what this reads (ADR-0035 point 12).
async fn lock(conn: &mut PgConnection, assignment_id: Uuid) -> Result<Assignment, AssessmentError> {
    content::lock_assignment_shared(conn, assignment_id)
        .await?
        .ok_or(AssessmentError::AssignmentNotFound)
}

/// Whether a save or a submit is taken at `now`, and whether it is late.
fn open_window(assignment: &Assignment, now: DateTime<Utc>) -> Result<bool, AssessmentError> {
    match Window::at(&assignment.settings, now) {
        Window::Open { late } => Ok(late),
        Window::Closed => Err(AssessmentError::WindowClosed),
    }
}

/// Replaces the caller's draft with `content`; the last write wins and nothing is versioned.
/// Writes no event (ADR-0007).
///
/// The assignment row is locked `FOR SHARE` before the settings are read from it, so a change of
/// the window made by a transaction that began first is seen, and one that comes later waits.
pub async fn save_draft(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    content: &Value,
    profiles: &EffectiveProfiles,
    now: DateTime<Utc>,
) -> Result<Draft, AssessmentError> {
    let opened = open(conn, actor, assignment_id).await?;
    policy::authorize(actor, Action::SubmitWork(opened.access))?;
    require_submissions(&opened)?;
    let assignment = lock(conn, assignment_id).await?;
    open_window(&assignment, now)?;
    let valid = assignment
        .assignment_type
        .validate_submission(&assignment.config, content, profiles)
        .map_err(AssessmentError::Document)?;
    Ok(repo::upsert_draft(conn, assignment_id, actor.user_id(), valid.as_body()).await?)
}

/// The caller's own work: the draft and a page of attempts, newest first.
pub async fn my_submission(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    page: PageParams,
) -> Result<MySubmission, AssessmentError> {
    let opened = open(conn, actor, assignment_id).await?;
    policy::authorize(actor, Action::ViewOwnSubmission(opened.access))?;
    require_submissions(&opened)?;
    let draft = repo::find_draft(conn, assignment_id, actor.user_id()).await?;
    let rows = repo::list_attempts(
        conn,
        assignment_id,
        actor.user_id(),
        page.after,
        i64::from(page.limit) + 1,
    )
    .await?;
    Ok(MySubmission {
        draft,
        attempts: Page::from_rows(rows, page.limit, |attempt| attempt.id),
    })
}

/// Deletes the caller's draft; with none it does nothing. Allowed after the window closed: it is
/// not a save (ADR-0035). Writes no event.
pub async fn discard_draft(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
) -> Result<(), AssessmentError> {
    let opened = open(conn, actor, assignment_id).await?;
    policy::authorize(actor, Action::SubmitWork(opened.access))?;
    require_submissions(&opened)?;
    lock(conn, assignment_id).await?;
    repo::delete_draft(conn, assignment_id, actor.user_id()).await?;
    Ok(())
}

/// Turns the caller's draft into their next attempt and emits `submission.submitted`.
///
/// Order: the assignment row is locked `FOR SHARE` and the window is judged on the settings in it;
/// then the draft row is locked `FOR UPDATE`, so a second submit of the same draft waits and then
/// finds no draft; then, in a later statement, the attempts are counted. The draft row becomes the
/// attempt in place.
pub async fn submit(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    profiles: &EffectiveProfiles,
    now: DateTime<Utc>,
) -> Result<Attempt, AssessmentError> {
    let opened = open(conn, actor, assignment_id).await?;
    policy::authorize(actor, Action::SubmitWork(opened.access))?;
    require_submissions(&opened)?;
    let assignment = lock(conn, assignment_id).await?;
    let is_late = open_window(&assignment, now)?;
    let student_id = actor.user_id();
    let draft = repo::lock_draft(conn, assignment_id, student_id)
        .await?
        .ok_or(AssessmentError::NoDraft)?;
    let last = repo::last_attempt_number(conn, assignment_id, student_id).await?;
    if assignment
        .settings
        .max_attempts
        .is_some_and(|max| last >= max)
    {
        return Err(AssessmentError::AttemptLimit);
    }
    // The draft is judged again against the profile and the features as they are now.
    let valid = assignment
        .assignment_type
        .validate_submission(&assignment.config, &Value::Object(draft.content), profiles)
        .map_err(AssessmentError::Document)?;
    if valid.is_blank() {
        return Err(AssessmentError::Empty);
    }
    let attempt = repo::submit_draft(conn, draft.id, last + 1, now, is_late).await?;
    events::emit(
        conn,
        Event::SubmissionSubmitted {
            assignment_id,
            student_id,
            course_id: opened.course_id,
            submission_id: attempt.id,
        },
    )
    .await?;
    Ok(attempt)
}

/// The current students of the course with their latest submitted attempt, in roster order.
/// Staff and admins. No draft is read.
pub async fn list_submissions(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    page: PageParams,
) -> Result<Page<StudentSubmission>, AssessmentError> {
    let opened = open(conn, actor, assignment_id).await?;
    policy::authorize(actor, Action::ViewSubmissions(opened.access))?;
    require_submissions(&opened)?;
    let rows = repo::list_students(
        conn,
        opened.course_id,
        assignment_id,
        page.after,
        i64::from(page.limit) + 1,
    )
    .await?;
    Ok(Page::from_rows(rows, page.limit, |row| row.student_id))
}

/// A page of the submitted attempts of one student, newest first. Staff and admins. No draft is
/// read.
pub async fn student_attempts(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    student_id: Uuid,
    page: PageParams,
) -> Result<Page<Attempt>, AssessmentError> {
    let opened = open(conn, actor, assignment_id).await?;
    policy::authorize(actor, Action::ViewSubmissions(opened.access))?;
    require_submissions(&opened)?;
    if !repo::is_student_or_has_attempts(conn, opened.course_id, assignment_id, student_id).await? {
        return Err(AssessmentError::StudentNotFound);
    }
    let rows = repo::list_attempts(
        conn,
        assignment_id,
        student_id,
        page.after,
        i64::from(page.limit) + 1,
    )
    .await?;
    Ok(Page::from_rows(rows, page.limit, |attempt| attempt.id))
}

// ---------------------------------------------------------------------------------------------
// Callbacks for `content` (and later `teaching`). The caller has authorized the actor and holds
// the lock of ADR-0035 point 12; none of these takes an `Actor`.
// ---------------------------------------------------------------------------------------------

/// True when the assignment has a submitted attempt.
pub async fn has_submitted_for_assignment(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<bool, AssessmentError> {
    Ok(repo::has_submitted_for_assignment(conn, assignment_id).await?)
}

/// True when an assignment in the module or its sub-modules has a submitted attempt.
pub async fn has_submitted_in_module(
    conn: &mut PgConnection,
    module_id: Uuid,
) -> Result<bool, AssessmentError> {
    Ok(repo::has_submitted_in_module(conn, module_id).await?)
}

/// True when an assignment of the course has a submitted attempt.
pub async fn has_submitted_in_course(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<bool, AssessmentError> {
    Ok(repo::has_submitted_in_course(conn, course_id).await?)
}

/// Deletes the drafts of the assignment, for the delete that follows in the same transaction.
/// Refuses with `HasSubmissions` instead of destroying an attempt. Returns how many drafts went.
pub async fn purge_assignment(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<u64, AssessmentError> {
    if repo::has_submitted_for_assignment(conn, assignment_id).await? {
        return Err(AssessmentError::HasSubmissions);
    }
    Ok(repo::purge_assignment(conn, assignment_id).await?)
}

/// Like [`purge_assignment`] for a module and its sub-modules.
pub async fn purge_module(
    conn: &mut PgConnection,
    module_id: Uuid,
) -> Result<u64, AssessmentError> {
    if repo::has_submitted_in_module(conn, module_id).await? {
        return Err(AssessmentError::HasSubmissions);
    }
    Ok(repo::purge_module(conn, module_id).await?)
}

/// Like [`purge_assignment`] for every assignment of a course.
pub async fn purge_course(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<u64, AssessmentError> {
    if repo::has_submitted_in_course(conn, course_id).await? {
        return Err(AssessmentError::HasSubmissions);
    }
    Ok(repo::purge_course(conn, course_id).await?)
}

#[cfg(test)]
mod tests {
    //! The clock comes from the caller, so the boundaries of the window are tested with explicit
    //! moments. Lock order (ADR-0035 point 12, READ COMMITTED): each concurrency test holds one
    //! transaction open, starts a second on a task, waits until that task is observed blocked on
    //! a lock, and only then lets the first one commit. Nothing sleeps: the order is forced by
    //! the locks, not by timing.
    use crate::test_support::{db_test, wait_until_blocked};

    use std::sync::LazyLock;

    use serde_json::{Map, json};
    use sqlx::PgPool;

    use super::*;
    use crate::content::{
        Assignment, BuiltinType, ContentError, ContentTitle, ItemPatch, ItemRules, MaxPoints,
        OnDeadline, SettingsPatch, SubmissionSettings,
    };
    use crate::identity::{self, Email, NewUser, PersonName, Role};
    use crate::policy::CourseRole;
    use crate::settings::GradingScale;
    use crate::teaching::{self, CourseTitle};

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
        let student = Actor::new(student_user.id, Role::Student, Uuid::now_v7());
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
            let (pool, owner, assignment) =
                (self.pool.clone(), self.owner.clone(), self.assignment);
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
}
