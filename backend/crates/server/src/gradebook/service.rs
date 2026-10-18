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

use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::PgConnection;
use sqlx::types::Decimal;
use uuid::Uuid;

use super::model::{
    Cell, Feedback, FeedbackInput, FieldProblem, GradeInput, GradebookPage, GradebookRow,
    HistoryEntry, ModelError, MyGrade, PublishedGrade, Score, Students, TeachingOverview,
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
        Some(actor.user_id()),
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

/// Writes the score a quiz earned as the draft score of the student's cell and records an
/// `auto_score` history entry with no person (ADR-0050 point 5). The draft feedback and the
/// published value stay. It authorizes nothing: `assessment` calls it from the submit of a current
/// student, which holds the assignment row `FOR SHARE`. Emits no event (ADR-0007).
pub async fn save_auto_grade(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    student_id: Uuid,
    score: Decimal,
) -> Result<(), GradebookError> {
    let max = lock_max_points(conn, assignment_id).await?;
    if score < Decimal::ZERO || score > max {
        return Err(GradebookError::Invalid(vec![FieldProblem {
            field: "score",
            message: "the quiz score is outside 0 to max_points".to_owned(),
        }]));
    }
    let feedback = repo::upsert_auto_score(conn, assignment_id, student_id, score).await?;
    repo::insert_history(
        conn,
        assignment_id,
        student_id,
        "auto_score",
        None,
        Some(score),
        feedback.as_deref(),
    )
    .await?;
    Ok(())
}

/// What was published for one student on one assignment, or `None` before anything was. It
/// authorizes nothing: the caller has shown that the reader is that student (ADR-0050 point 8).
pub async fn published_grade(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    student_id: Uuid,
) -> Result<Option<PublishedGrade>, GradebookError> {
    Ok(repo::find_published(conn, assignment_id, student_id).await?)
}

/// What the caller has to do across the courses they teach, assist in or own and that are not
/// archived (ADR-0052). The lists are bounded by what one person teaches; the queue of work to
/// grade is cut at [`repo::TO_GRADE_LIMIT`].
pub async fn teaching_overview(
    conn: &mut PgConnection,
    actor: &Actor,
    now: DateTime<Utc>,
) -> Result<TeachingOverview, GradebookError> {
    let courses: Vec<_> = teaching::course_scope(conn, actor)
        .await
        .map_err(GradebookError::Teaching)?
        .into_iter()
        .filter(|scoped| scoped.staff && scoped.course.course.status != CourseStatus::Archived)
        .map(|scoped| scoped.course)
        .collect();
    let ids: Vec<Uuid> = courses.iter().map(|card| card.course.id).collect();
    Ok(TeachingOverview {
        to_grade: repo::to_grade(conn, &ids).await?,
        unpublished: repo::unpublished_grades(conn, &ids).await?,
        deadlines: repo::deadlines(conn, &ids, now).await?,
        unpublished_content: repo::unpublished_content(conn, &ids).await?,
        courses,
    })
}

/// The recorded changes of one student's grade, newest first (ADR-0049). Staff only; a student
/// with no grade has an empty history.
pub async fn grade_history(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    student_id: Uuid,
    page: PageParams,
) -> Result<Page<HistoryEntry>, GradebookError> {
    let context = open_assignment(conn, actor, assignment_id).await?;
    policy::authorize(actor, Action::ViewGradebook(context.access))?;
    let rows = repo::list_history(
        conn,
        assignment_id,
        student_id,
        page.after,
        i64::from(page.limit) + 1,
    )
    .await?;
    Ok(Page::from_rows(rows, page.limit, |entry| entry.id))
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
            Some(actor.user_id()),
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
            Some(_) => GradebookError::Forbidden(PolicyError::Denied),
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
mod tests;
