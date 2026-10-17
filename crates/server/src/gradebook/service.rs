//! Business rules. Every function that acts for a user authorizes first, takes the connection it
//! runs on and never begins or commits a transaction.
//!
//! An assignment the actor may not see does not exist as far as they can tell (404): an unknown
//! id, an id in a course they cannot see and a draft they may not open give the same error. Only
//! after that does the action decide between going ahead and 403.
//!
//! `gradebook` calls no other module's Rust code except the read-only functions that every module
//! uses to open and lock an assignment: `teaching::course_access` and
//! `content::{open_assignment, lock_assignment_shared}` (ADR-0028, ADR-0030, ADR-0035). The
//! tables of `content`, `teaching` and `identity` are otherwise read in `repo` with JOINs.

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use serde_json::json;
use sqlx::PgConnection;
use sqlx::types::Decimal;
use uuid::Uuid;

use super::model::{
    Cell, Feedback, FeedbackInput, FieldProblem, GradeInput, GradebookPage, GradebookRow,
    ModelError, MyGrade, Score, Students,
};
use super::repo;
use crate::actor::Actor;
use crate::content::{self, OpenError, OpenedAssignment};
use crate::events::{self, AuditEntry, Event, EventsError};
use crate::pagination::{Page, PageParams};
use crate::policy::{self, Action, CourseAccess, CourseRole, CourseStatus, PolicyError};
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
    /// The student left the roster; their grades are kept but cannot be written (ADR-0037).
    #[error("the student is no longer on the roster")]
    StudentFormer,
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
) -> Result<OpenedAssignment, GradebookError> {
    content::open_assignment(conn, actor, assignment_id)
        .await
        .map_err(|error| match error {
            OpenError::NotFound => GradebookError::AssignmentNotFound,
            OpenError::Teaching(error) => GradebookError::Teaching(error),
            OpenError::Db(error) => GradebookError::Db(error),
        })
}

/// Takes the assignment row `FOR SHARE` and returns `max_points` as it is in that row.
async fn lock_max_points(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<Decimal, GradebookError> {
    content::lock_assignment_shared(conn, assignment_id)
        .await?
        .map(|assignment| assignment.max_points.value())
        .ok_or(GradebookError::AssignmentNotFound)
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
    let context = open_assignment(conn, actor, assignment_id).await?;
    policy::authorize(actor, Action::GradeAssignment(context.access))?;
    let max = lock_max_points(conn, assignment_id).await?;
    if !repo::is_student_member(conn, context.course_id, student_id).await? {
        return Err(
            if repo::has_grades_of_student(conn, context.course_id, student_id).await? {
                GradebookError::StudentFormer
            } else {
                GradebookError::StudentNotFound
            },
        );
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
    let context = open_assignment(conn, actor, assignment_id).await?;
    policy::authorize(actor, Action::PublishGrades(context.access))?;
    lock_max_points(conn, assignment_id).await?;
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
    // One map for the whole page, so each cell is found by its key instead of by a search.
    let mut cells: HashMap<(Uuid, Uuid), Cell> = repo::cells(conn, course_id, &ids)
        .await?
        .into_iter()
        .map(|row| ((row.student_id, row.assignment_id), row.cell))
        .collect();
    let rows = students
        .items
        .into_iter()
        .map(|student| {
            // In column order, so a client can zip cells with columns.
            let own: Vec<(Uuid, Cell)> = columns
                .iter()
                .filter_map(|column| {
                    let cell = cells.remove(&(student.user_id, column.assignment_id))?;
                    Some((column.assignment_id, cell))
                })
                .collect();
            GradebookRow {
                user_id: student.user_id,
                name: student.name,
                surname: student.surname,
                former: student.former,
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
/// caller may see now. Someone who is not on the roster any more (left the group, was disabled and
/// enabled again, was promoted, was excluded) still reads their own published grades, and nothing
/// else of the course (ADR-0037).
pub async fn my_grades(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<Vec<MyGrade>, GradebookError> {
    let access = match course_access(conn, actor, course_id).await {
        Ok(access) => Some(access),
        Err(GradebookError::CourseNotFound) => None,
        Err(other) => return Err(other),
    };
    if let Some(access) = access
        && policy::authorize(actor, Action::ViewOwnGrades(access)).is_ok()
    {
        return published_grades(conn, actor, course_id, access).await;
    }
    // Not a current student: the grades of a former one, if there are any.
    let has_rows = repo::has_published_grades(conn, course_id, actor.user_id()).await?;
    if !has_rows {
        return Err(match access {
            Some(_) => GradebookError::Forbidden(PolicyError),
            None => GradebookError::CourseNotFound,
        });
    }
    policy::authorize(actor, Action::ViewFormerGrades { has_rows })?;
    // The rows are the user's own, so the course and its status do not matter any more; each
    // assignment is still shown only while its module and item are open.
    let access = CourseAccess {
        role: Some(CourseRole::Student),
        is_owner: false,
        status: CourseStatus::Published,
    };
    published_grades(conn, actor, course_id, access).await
}

async fn published_grades(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    access: CourseAccess,
) -> Result<Vec<MyGrade>, GradebookError> {
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
    use crate::test_support::{db_test, wait_until_blocked};

    use std::sync::LazyLock;

    use serde_json::Map;
    use sqlx::PgPool;

    use super::*;
    use crate::content::{
        self, Assignment, BuiltinType, ContentError, ContentTitle, EffectiveProfiles, ItemPatch,
        ItemRules, MaxPoints, SubmissionSettings,
    };
    use crate::identity::{self, Email, NewUser, PersonName, Role};
    use crate::pagination::PageParams;
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

        /// Publishes every student's draft in its own transaction on a task and commits when it
        /// succeeds.
        fn publish_in_background(
            &self,
        ) -> tokio::task::JoinHandle<Result<Vec<Uuid>, GradebookError>> {
            let (pool, owner, assignment) =
                (self.pool.clone(), self.owner.clone(), self.assignment);
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
}
