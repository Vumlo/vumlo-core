//! The work read models: the facts of the items of a course, and a student's work list
//! (ADR-0049, ADR-0052). They sit in `gradebook` because they read grades and attempts, which
//! the content below them does not know.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use super::{GradebookError, course_access};
use crate::model::{ItemMeta, MyWork, Progress, TodoCounts, WorkEntry, WorkFacts};
use crate::repo;
use access::{self, Action, Actor};
use content::{self, NextDeadline};

/// The facts of every item of the course that the actor may see (ADR-0049, point 5): what
/// `content` knows of each item, plus the counts of what to grade for staff, or a student's own
/// standing (published values only). Not paged: a course tree is bounded (ADR-0020).
pub async fn item_meta(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<HashMap<Uuid, ItemMeta>, GradebookError> {
    let access = course_access(conn, actor, course_id).await?;
    let mut meta: HashMap<Uuid, ItemMeta> = content::item_meta(conn, actor, course_id)
        .await?
        .into_iter()
        .map(|(id, content)| {
            (
                id,
                ItemMeta {
                    content,
                    ..ItemMeta::default()
                },
            )
        })
        .collect();
    // The counts of what to grade are about submissions: a unit viewer without `view_grades` sees
    // the drafts of the content, not them.
    if access::authorize(actor, Action::ViewSubmissions(access)).is_ok() {
        for counts in repo::staff_counts(conn, course_id).await? {
            if let Some(entry) = meta.get_mut(&counts.assignment_id) {
                entry.staff = Some(counts);
            }
        }
    } else if access.is_student() {
        for work in repo::student_work(conn, actor.user_id(), &[course_id], Utc::now()).await? {
            if let Some(entry) = meta.get_mut(&work.assignment_id) {
                entry.work = Some(work);
            }
        }
    }
    Ok(meta)
}

/// A student's progress per course of `course_ids`, over what they can open (published values
/// only). The caller has authorized: it passes the courses the actor studies in.
pub async fn student_progress(
    conn: &mut PgConnection,
    student_id: Uuid,
    course_ids: &[Uuid],
    now: DateTime<Utc>,
) -> Result<Vec<Progress>, GradebookError> {
    Ok(repo::student_progress(conn, student_id, course_ids, now).await?)
}

/// The earliest upcoming deadline per course of `course_ids` of work the student has not handed
/// in. The caller has authorized: it passes the courses the actor studies in.
pub async fn student_next(
    conn: &mut PgConnection,
    student_id: Uuid,
    course_ids: &[Uuid],
    now: DateTime<Utc>,
) -> Result<Vec<NextDeadline>, GradebookError> {
    Ok(repo::student_next(conn, student_id, course_ids, now).await?)
}

/// What staff still have to grade per course of `course_ids`. Reads drafts: the caller has
/// authorized and passes only the courses the actor works in as staff.
pub async fn todo_counts(
    conn: &mut PgConnection,
    course_ids: &[Uuid],
) -> Result<Vec<TodoCounts>, GradebookError> {
    Ok(repo::todo_counts(conn, course_ids).await?)
}

/// Every assignment a student can open in their courses with their own standing on it, and the
/// courses (ADR-0052). Published values only. Not paged: a student has a bounded number of
/// courses and a course tree is bounded (ADR-0020).
pub async fn my_work(
    conn: &mut PgConnection,
    actor: &Actor,
    now: DateTime<Utc>,
) -> Result<MyWork, GradebookError> {
    let courses = teaching::student_courses(conn, actor)
        .await
        .map_err(GradebookError::Teaching)?;
    let ids: Vec<Uuid> = courses.iter().map(|card| card.course.id).collect();
    let standing: HashMap<Uuid, WorkFacts> = repo::student_work(conn, actor.user_id(), &ids, now)
        .await?
        .into_iter()
        .map(|facts| (facts.assignment_id, facts))
        .collect();
    let items = content::open_assignments(conn, &ids, now)
        .await?
        .into_iter()
        .filter_map(|row| {
            let facts = standing.get(&row.assignment_id)?;
            Some(WorkEntry {
                assignment_id: row.assignment_id,
                course_id: row.course_id,
                title: row.title,
                assignment_type: row.assignment_type,
                due_at: row.due_at,
                class_on: row.class_on,
                max_points: row.max_points,
                status: facts.status,
                score: facts.score,
                published_at: facts.published_at,
                draft_saved_at: facts.draft_saved_at,
            })
        })
        .collect();
    Ok(MyWork { courses, items })
}
