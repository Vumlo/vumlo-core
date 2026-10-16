//! Business rules. Every function that acts for a user authorizes first, takes the connection it
//! runs on and never begins or commits a transaction.
//!
//! An assignment the actor may not see does not exist as far as they can tell (404): an unknown
//! id, an id in a course they cannot see and a draft they may not open give the same error. Only
//! after that does the action decide between going ahead and 403.
//!
//! `gradebook` calls no other module's Rust code except the read-only course-access function of
//! `teaching` (ADR-0028, ADR-0030): the tables of `content`, `teaching` and `identity` are read in
//! `repo` with JOINs.

use std::collections::HashSet;

use chrono::Utc;
use serde_json::json;
use sqlx::PgConnection;
use sqlx::types::Decimal;
use uuid::Uuid;

use super::model::{
    Cell, Feedback, FeedbackInput, FieldProblem, GradeInput, GradebookPage, GradebookRow,
    ModelError, MyGrade, Score, Students,
};
use super::repo::{self, AssignmentContext};
use crate::actor::Actor;
use crate::events::{self, AuditEntry, Event, EventsError};
use crate::pagination::{Page, PageParams};
use crate::policy::{self, Action, CourseAccess, PolicyError};
use crate::teaching::{self, TeachingError};

#[derive(Debug, thiserror::Error)]
pub enum GradebookError {
    #[error(transparent)]
    Forbidden(#[from] PolicyError),
    /// A grade write with invalid values; one entry per rejected field.
    #[error("the request has invalid fields")]
    Invalid(Vec<FieldProblem>),
    /// No such assignment, or the actor may not see it.
    #[error("assignment not found")]
    AssignmentNotFound,
    /// No such course, or the actor may not see it.
    #[error("course not found")]
    CourseNotFound,
    /// The student is not a current student member of the course.
    #[error("student not found")]
    StudentNotFound,
    /// The positions, in the request, of the listed students who are not current students.
    #[error("not a current student of the course")]
    StudentsNotInCourse(Vec<usize>),
    /// A purge found grades with a value: the caller had to refuse the delete instead.
    #[error("there are grades")]
    HasGrades,
    #[error(transparent)]
    Teaching(TeachingError),
    #[error(transparent)]
    Events(#[from] EventsError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// How `actor` relates to the course. A course the actor may not view is `CourseNotFound`.
async fn course_access(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<CourseAccess, GradebookError> {
    teaching::course_access(conn, actor, course_id)
        .await
        .map_err(|error| match error {
            TeachingError::CourseNotFound => GradebookError::CourseNotFound,
            other => GradebookError::Teaching(other),
        })
}

/// Loads an assignment the actor may view, with their access to its course.
async fn open_assignment(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
) -> Result<(AssignmentContext, CourseAccess), GradebookError> {
    let context = repo::find_assignment(conn, assignment_id)
        .await?
        .ok_or(GradebookError::AssignmentNotFound)?;
    let access = course_access(conn, actor, context.course_id)
        .await
        .map_err(|error| match error {
            GradebookError::CourseNotFound => GradebookError::AssignmentNotFound,
            other => other,
        })?;
    // A module with a parent is a sub-module: the parent is the top-level module on the path.
    let (module, submodule) = match context.parent {
        Some(parent) => (parent, Some(context.module)),
        None => (context.module, None),
    };
    let view = Action::ViewContent {
        access,
        module,
        submodule,
        item: Some(context.item),
        at: Utc::now(),
    };
    policy::authorize(actor, view).map_err(|_| GradebookError::AssignmentNotFound)?;
    Ok((context, access))
}

async fn audit(
    conn: &mut PgConnection,
    actor: &Actor,
    action: &'static str,
    assignment_id: Uuid,
    meta: serde_json::Value,
) -> Result<(), GradebookError> {
    events::audit_admin(
        conn,
        actor,
        AuditEntry {
            action,
            resource_type: "assignment",
            resource_id: assignment_id,
            meta,
        },
    )
    .await?;
    Ok(())
}

/// Judges the raw values of a draft cell against the assignment's `max_points`.
fn validate(
    input: GradeInput,
    max: Decimal,
) -> Result<(Option<Decimal>, Option<Feedback>), GradebookError> {
    let mut problems = Vec::new();
    let score = match input.score.as_deref().map(|raw| Score::parse(raw, max)) {
        None => None,
        Some(Ok(score)) => Some(score.value()),
        Some(Err(error)) => {
            problems.push(FieldProblem {
                field: "score",
                message: error.to_string(),
            });
            None
        }
    };
    let feedback = match input.feedback {
        None => None,
        Some(FeedbackInput::Text(raw)) => Feedback::parse(&raw).unwrap_or_else(|error| {
            problems.push(FieldProblem {
                field: "feedback",
                message: error.to_string(),
            });
            None
        }),
        Some(FeedbackInput::NotText) => {
            problems.push(FieldProblem {
                field: "feedback",
                message: ModelError::Feedback.to_string(),
            });
            None
        }
    };
    if problems.is_empty() {
        Ok((score, feedback))
    } else {
        Err(GradebookError::Invalid(problems))
    }
}

/// Replaces the draft cell of one student; `None` clears a field. Writes `draft_*` only, appends
/// to the history and emits no event (ADR-0007).
///
/// The assignment row is locked `FOR SHARE` before `max_points` is read from it, so a change of
/// `max_points` or a delete of the assignment waits for this transaction and then sees the
/// score. The student is checked in a later statement.
pub async fn save_grade(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    student_id: Uuid,
    input: GradeInput,
) -> Result<Cell, GradebookError> {
    let (context, access) = open_assignment(conn, actor, assignment_id).await?;
    policy::authorize(actor, Action::GradeAssignment(access))?;
    let max = repo::lock_assignment_shared(conn, assignment_id)
        .await?
        .ok_or(GradebookError::AssignmentNotFound)?;
    if !repo::is_student_member(conn, context.course_id, student_id).await? {
        return Err(GradebookError::StudentNotFound);
    }
    let (score, feedback) = validate(input, max)?;
    let feedback = feedback.as_ref().map(Feedback::as_str);
    let cell = repo::upsert_draft(
        conn,
        assignment_id,
        student_id,
        score,
        feedback,
        actor.user_id(),
    )
    .await?;
    repo::insert_history(
        conn,
        assignment_id,
        student_id,
        "save_draft",
        actor.user_id(),
        score,
        feedback,
    )
    .await?;
    audit(
        conn,
        actor,
        "grade.save",
        assignment_id,
        json!({"course_id": context.course_id, "student_id": student_id}),
    )
    .await?;
    Ok(cell)
}

/// Copies draft to published for the students of the course, in this transaction, only where the
/// draft differs from the published value. Returns the students whose published value was
/// replaced, a retraction included. A student whose published value became non-empty gets one
/// `grade.published` event; a retraction emits none (ADR-0028).
pub async fn publish_grades(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    students: Students,
) -> Result<Vec<Uuid>, GradebookError> {
    let (context, access) = open_assignment(conn, actor, assignment_id).await?;
    policy::authorize(actor, Action::PublishGrades(access))?;
    repo::lock_assignment_shared(conn, assignment_id)
        .await?
        .ok_or(GradebookError::AssignmentNotFound)?;
    let only = match students {
        Students::All => None,
        Students::Listed(mut ids) => {
            let members: HashSet<Uuid> = repo::student_members_among(conn, context.course_id, &ids)
                .await?
                .into_iter()
                .collect();
            let outside: Vec<usize> = ids
                .iter()
                .enumerate()
                .filter(|(_, id)| !members.contains(id))
                .map(|(position, _)| position)
                .collect();
            if !outside.is_empty() {
                return Err(GradebookError::StudentsNotInCourse(outside));
            }
            ids.sort_unstable();
            ids.dedup();
            Some(ids)
        }
    };
    let locked =
        repo::lock_publishable(conn, assignment_id, context.course_id, only.as_deref()).await?;
    if locked.is_empty() {
        return Ok(Vec::new());
    }
    let published = repo::publish_rows(conn, assignment_id, &locked, actor.user_id()).await?;
    for row in &published {
        repo::insert_history(
            conn,
            assignment_id,
            row.student_id,
            "publish",
            actor.user_id(),
            row.score,
            row.feedback.as_deref(),
        )
        .await?;
        if row.score.is_some() || row.feedback.is_some() {
            events::emit(
                conn,
                Event::GradePublished {
                    assignment_id,
                    student_id: row.student_id,
                    course_id: context.course_id,
                },
            )
            .await?;
        }
    }
    let student_ids: Vec<Uuid> = published.iter().map(|row| row.student_id).collect();
    if !student_ids.is_empty() {
        audit(
            conn,
            actor,
            "grade.publish",
            assignment_id,
            json!({"course_id": context.course_id, "student_ids": student_ids}),
        )
        .await?;
    }
    Ok(student_ids)
}

/// One page of the staff gradebook: the columns of the course and, for a page of its current
/// students, their cells.
pub async fn gradebook(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    page: PageParams,
) -> Result<GradebookPage, GradebookError> {
    let access = course_access(conn, actor, course_id).await?;
    policy::authorize(actor, Action::ViewGradebook(access))?;
    let columns = repo::columns(conn, course_id).await?;
    let students = repo::students(conn, course_id, page.after, i64::from(page.limit) + 1).await?;
    let students = Page::from_rows(students, page.limit, |student| student.user_id);
    let ids: Vec<Uuid> = students
        .items
        .iter()
        .map(|student| student.user_id)
        .collect();
    let mut cells = repo::cells(conn, course_id, &ids).await?;
    let rows = students
        .items
        .into_iter()
        .map(|student| {
            // In column order, so a client can zip cells with columns.
            let own: Vec<(Uuid, Cell)> = columns
                .iter()
                .filter_map(|column| {
                    let at = cells.iter().position(|cell| {
                        cell.student_id == student.user_id
                            && cell.assignment_id == column.assignment_id
                    })?;
                    let row = cells.swap_remove(at);
                    Some((row.assignment_id, row.cell))
                })
                .collect();
            GradebookRow {
                user_id: student.user_id,
                name: student.name,
                surname: student.surname,
                cells: own,
            }
        })
        .collect();
    Ok(GradebookPage {
        columns,
        rows,
        next: students.next,
    })
}

/// The caller's published grades in the course: only published values, only for assignments the
/// caller may see now.
pub async fn my_grades(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<Vec<MyGrade>, GradebookError> {
    let access = course_access(conn, actor, course_id).await?;
    policy::authorize(actor, Action::ViewOwnGrades(access))?;
    let now = Utc::now();
    let rows = repo::my_published_grades(conn, course_id, actor.user_id(), now).await?;
    Ok(rows
        .into_iter()
        .filter(|row| {
            let (module, submodule) = match row.parent {
                Some(parent) => (parent, Some(row.module)),
                None => (row.module, None),
            };
            let view = Action::ViewContent {
                access,
                module,
                submodule,
                item: Some(row.item),
                at: now,
            };
            policy::authorize(actor, view).is_ok()
        })
        .map(|row| row.grade)
        .collect())
}

// ---------------------------------------------------------------------------------------------
// Callbacks for `content` (and later `teaching`). The caller has authorized the actor and holds
// the lock of ADR-0028 point 5; none of these takes an `Actor`.
// ---------------------------------------------------------------------------------------------

/// The highest draft or published score of the assignment, `None` when it has none.
pub async fn max_score(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<Option<Decimal>, GradebookError> {
    Ok(repo::max_score(conn, assignment_id).await?)
}

/// True when the assignment has a grade with a non-empty draft or published value.
pub async fn has_grades_for_assignment(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<bool, GradebookError> {
    Ok(repo::has_grades_for_assignment(conn, assignment_id).await?)
}

/// True when an assignment in the module or its sub-modules has a non-empty grade.
pub async fn has_grades_in_module(
    conn: &mut PgConnection,
    module_id: Uuid,
) -> Result<bool, GradebookError> {
    Ok(repo::has_grades_in_module(conn, module_id).await?)
}

/// True when an assignment of the course has a non-empty grade.
pub async fn has_grades_in_course(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<bool, GradebookError> {
    Ok(repo::has_grades_in_course(conn, course_id).await?)
}

/// Deletes the empty grade rows of the assignment and their history, for the delete that follows
/// in the same transaction. Refuses with `HasGrades` instead of destroying a value. Returns how
/// many rows were deleted.
pub async fn purge_assignment(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<u64, GradebookError> {
    if repo::has_grades_for_assignment(conn, assignment_id).await? {
        return Err(GradebookError::HasGrades);
    }
    Ok(repo::purge_assignment(conn, assignment_id).await?)
}

/// Like [`purge_assignment`] for a module and its sub-modules.
pub async fn purge_module(conn: &mut PgConnection, module_id: Uuid) -> Result<u64, GradebookError> {
    if repo::has_grades_in_module(conn, module_id).await? {
        return Err(GradebookError::HasGrades);
    }
    Ok(repo::purge_module(conn, module_id).await?)
}

/// Like [`purge_assignment`] for every assignment of a course.
pub async fn purge_course(conn: &mut PgConnection, course_id: Uuid) -> Result<u64, GradebookError> {
    if repo::has_grades_in_course(conn, course_id).await? {
        return Err(GradebookError::HasGrades);
    }
    Ok(repo::purge_course(conn, course_id).await?)
}

#[cfg(test)]
mod tests {
    //! Lock order (ADR-0028 point 5, READ COMMITTED). Each test holds one writer's transaction
    //! open, starts a second writer on a task, waits until that writer is observed blocked on a
    //! lock, and only then lets the first one commit. Nothing sleeps: the order is forced by the
    //! locks, not by timing.

    use std::sync::LazyLock;
    use std::time::Duration;

    use serde_json::Map;
    use sqlx::PgPool;

    use super::*;
    use crate::content::{
        self, Assignment, BuiltinType, ContentError, ContentTitle, EffectiveProfiles, ItemPatch,
        ItemRules, MaxPoints,
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

    struct Fixture {
        pool: PgPool,
        owner: Actor,
        student: Uuid,
        assignment: Uuid,
    }

    /// A course with a teacher, a student and one assignment worth 10 points.
    async fn fixture(pool: PgPool) -> Fixture {
        let hash = identity::hash_password("irrelevant").await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        let mut create = async |email: &str, role: Role| {
            identity::create_user(
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
        let course =
            teaching::create_course(&mut tx, &owner, CourseTitle::parse("Algebra").unwrap())
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
                max_points: MaxPoints::parse("10").unwrap(),
                is_bonus: false,
            }),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        Fixture {
            pool,
            owner,
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
            let (pool, owner, assignment) =
                (self.pool.clone(), self.owner.clone(), self.assignment);
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
            let (pool, owner, assignment) =
                (self.pool.clone(), self.owner.clone(), self.assignment);
            tokio::spawn(async move {
                let mut tx = pool.begin().await.unwrap();
                let result = content::delete_item(&mut tx, &owner, assignment).await;
                if result.is_ok() {
                    tx.commit().await.unwrap();
                }
                result
            })
        }

        async fn count(&self, sql: &'static str) -> i64 {
            sqlx::query_scalar::<_, i64>(sql)
                .bind(self.assignment)
                .fetch_one(&self.pool)
                .await
                .unwrap()
        }

        async fn grades(&self) -> i64 {
            self.count("SELECT count(*) FROM gradebook.grades WHERE assignment_id = $1")
                .await
        }

        async fn max_points(&self) -> String {
            sqlx::query_scalar::<_, String>(
                "SELECT max_points::text FROM content.assignments WHERE item_id = $1",
            )
            .bind(self.assignment)
            .fetch_one(&self.pool)
            .await
            .unwrap()
        }
    }

    /// Returns once a session waits for a lock. Fails if the writer finished instead (it did not
    /// wait for anything) or if nobody waits within the timeout.
    async fn wait_until_blocked<T>(pool: &PgPool, writer: &tokio::task::JoinHandle<T>) {
        let mut conn = pool.acquire().await.unwrap();
        let waited = tokio::time::timeout(Duration::from_secs(10), async {
            while !writer.is_finished() && repo::lock_waiters(&mut conn).await.unwrap() < 1 {
                tokio::task::yield_now().await;
            }
        })
        .await;
        assert!(waited.is_ok(), "no writer waited for a lock");
        assert!(
            !writer.is_finished(),
            "the second writer finished without waiting for the assignment row"
        );
    }

    /// A grade write is in flight with an uncommitted score of 8. A lowering to 5 must wait for
    /// it and then see the score. Without the row locks it would not see it and lower the points
    /// under a score of 8.
    #[sqlx::test(migrator = "crate::app::MIGRATOR")]
    async fn a_lowering_waits_for_a_grade_write_and_then_sees_its_score(pool: PgPool) {
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
    }

    /// A lowering to 5 is in flight and holds the row. A grade write of 8, valid against the old
    /// maximum, must wait for it and be checked against the new one. Without the lock the write
    /// would read 10, store 8 and leave a score above `max_points`.
    #[sqlx::test(migrator = "crate::app::MIGRATOR")]
    async fn a_grade_write_waits_for_a_lowering_and_is_checked_against_the_new_maximum(
        pool: PgPool,
    ) {
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
    }

    /// A grade write is in flight. A delete of the assignment must wait for it and then see the
    /// grade. Without the lock the delete would pass its check, remove the assignment and leave
    /// the grade behind with nothing to belong to.
    #[sqlx::test(migrator = "crate::app::MIGRATOR")]
    async fn a_delete_waits_for_a_grade_write_and_then_sees_the_grade(pool: PgPool) {
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
        assert_eq!(
            f.count("SELECT count(*) FROM content.assignments WHERE item_id = $1")
                .await,
            1
        );
    }

    /// A delete of the assignment is in flight. A grade write must wait for it and then find
    /// that the assignment is gone, instead of storing a grade for it.
    #[sqlx::test(migrator = "crate::app::MIGRATOR")]
    async fn a_grade_write_waits_for_a_delete_and_then_finds_the_assignment_gone(pool: PgPool) {
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
        assert_eq!(
            f.count("SELECT count(*) FROM content.items WHERE id = $1")
                .await,
            0
        );
    }
}
