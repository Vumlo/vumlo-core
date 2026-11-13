//! SQL of the work read models: what a student sees of their own standing and what staff still
//! have to grade (ADR-0049, ADR-0052). The student queries select published grade columns only;
//! the staff queries (`staff_counts`, `todo_counts`) read the draft columns.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::gradebook::model::{Progress, StaffFacts, TodoCounts, WorkFacts, WorkStatus};
use content::{AssignmentType, BuiltinType, NextDeadline};

/// A student's standing on every assignment of the given courses (see [`WorkStatus::of`]). Only
/// published grade columns are selected: nothing here can show a draft value (AGENTS section 6).
/// The caller keeps the assignments the student may see.
pub async fn student_work(
    conn: &mut PgConnection,
    student_id: Uuid,
    course_ids: &[Uuid],
    now: DateTime<Utc>,
) -> Result<Vec<WorkFacts>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT a.item_id AS "assignment_id!", i.course_id AS "course_id!",
                  a.type AS "assignment_type!: BuiltinType",
                  COALESCE(a.due_at IS NOT NULL
                           AND CASE WHEN a.accept_late THEN a.late_until ELSE a.due_at END < $3,
                           false) AS "closed!",
                  (SELECT s.updated_at FROM assessment.submission_index s
                   WHERE s.assignment_id = a.item_id AND s.student_id = $1
                     AND s.status = 'draft') AS draft_saved_at,
                  (SELECT count(*) FROM assessment.submission_index s
                   WHERE s.assignment_id = a.item_id AND s.student_id = $1
                     AND s.status = 'submitted') AS "attempts!",
                  g.published_score,
                  (g.published_score IS NOT NULL OR g.published_feedback IS NOT NULL)
                      AS "graded!",
                  g.published_at
           FROM content.assignment_list a
           JOIN content.item_list i ON i.id = a.item_id
           LEFT JOIN gradebook.grades g ON g.assignment_id = a.item_id AND g.student_id = $1
           WHERE i.course_id = ANY($2)"#,
        student_id,
        course_ids,
        now
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| WorkFacts {
            assignment_id: row.assignment_id,
            course_id: row.course_id,
            status: WorkStatus::of(
                row.graded,
                row.assignment_type.takes_submissions(),
                row.attempts,
                row.draft_saved_at.is_some(),
                row.closed,
            ),
            score: row.published_score,
            published_at: row.published_at,
            draft_saved_at: row.draft_saved_at,
            attempts_used: row.attempts,
        })
        .collect())
}

/// What staff still have to do on each assignment of a course, over the current students. Reads the
/// draft grade columns, so only a staff path may call it.
pub async fn staff_counts(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<Vec<StaffFacts>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT a.item_id AS "assignment_id!",
                  (SELECT count(*) FROM teaching.effective_members m
                   WHERE m.course_id = $1 AND m.role = 'student') AS "graded_total!",
                  (SELECT count(DISTINCT s.student_id) FROM assessment.submission_index s
                   JOIN teaching.effective_members m
                     ON m.course_id = $1 AND m.user_id = s.student_id AND m.role = 'student'
                   WHERE s.assignment_id = a.item_id AND s.status = 'submitted') AS "submitted!",
                  (SELECT count(DISTINCT s.student_id) FROM assessment.submission_index s
                   JOIN teaching.effective_members m
                     ON m.course_id = $1 AND m.user_id = s.student_id AND m.role = 'student'
                   LEFT JOIN gradebook.grades g
                     ON g.assignment_id = s.assignment_id AND g.student_id = s.student_id
                   WHERE s.assignment_id = a.item_id AND s.status = 'submitted'
                     AND g.draft_score IS NULL) AS "to_grade!",
                  (SELECT count(*) FROM gradebook.grades g
                   JOIN teaching.effective_members m
                     ON m.course_id = $1 AND m.user_id = g.student_id AND m.role = 'student'
                   WHERE g.assignment_id = a.item_id
                     AND (g.draft_score, g.draft_feedback)
                         IS DISTINCT FROM (g.published_score, g.published_feedback))
                      AS "draft_grades!"
           FROM content.assignment_list a
           JOIN content.item_list i ON i.id = a.item_id
           WHERE i.course_id = $1"#,
        course_id
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| StaffFacts {
            assignment_id: row.assignment_id,
            submitted: row.submitted,
            to_grade: row.to_grade,
            draft_grades: row.draft_grades,
            graded_total: row.graded_total,
        })
        .collect())
}

/// A student's numbers per course over the assignments that take submissions and that students can
/// open now.
pub async fn student_progress(
    conn: &mut PgConnection,
    student_id: Uuid,
    course_ids: &[Uuid],
    now: DateTime<Utc>,
) -> Result<Vec<Progress>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"WITH open AS (
               SELECT i.course_id, a.item_id,
                      (g.published_score IS NOT NULL OR g.published_feedback IS NOT NULL) AS graded,
                      EXISTS (SELECT 1 FROM assessment.submission_index s
                              WHERE s.assignment_id = a.item_id AND s.student_id = $1
                                AND s.status = 'submitted') AS attempted
               FROM content.assignment_list a
               JOIN content.item_list i ON i.id = a.item_id
               JOIN content.module_list m ON m.id = i.module_id
               LEFT JOIN content.module_list p ON p.id = m.parent_id
               LEFT JOIN gradebook.grades g ON g.assignment_id = a.item_id AND g.student_id = $1
               WHERE i.course_id = ANY($2) AND a.type <> 'no_submission'
                 AND i.status = 'published' AND (i.publish_at IS NULL OR i.publish_at <= $3)
                 AND m.status = 'published' AND (m.publish_at IS NULL OR m.publish_at <= $3)
                 AND (p.id IS NULL
                      OR (p.status = 'published' AND (p.publish_at IS NULL OR p.publish_at <= $3))))
           SELECT course_id AS "course_id!", count(*) AS "total!",
                  count(*) FILTER (WHERE attempted OR COALESCE(graded, false)) AS "submitted!",
                  count(*) FILTER (WHERE COALESCE(graded, false)) AS "graded!"
           FROM open GROUP BY course_id"#,
        student_id,
        course_ids,
        now
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Progress {
            course_id: row.course_id,
            total: row.total,
            submitted: row.submitted,
            graded: row.graded,
        })
        .collect())
}

/// The earliest upcoming deadline per course of work the student has not handed in.
pub async fn student_next(
    conn: &mut PgConnection,
    student_id: Uuid,
    course_ids: &[Uuid],
    now: DateTime<Utc>,
) -> Result<Vec<NextDeadline>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT DISTINCT ON (i.course_id) i.course_id AS "course_id!",
                  a.item_id AS "assignment_id!",
                  i.published_title AS "title!", a.due_at AS "due_at!"
           FROM content.assignment_list a
           JOIN content.item_list i ON i.id = a.item_id
           JOIN content.module_list m ON m.id = i.module_id
           LEFT JOIN content.module_list p ON p.id = m.parent_id
           LEFT JOIN gradebook.grades g ON g.assignment_id = a.item_id AND g.student_id = $1
           WHERE i.course_id = ANY($2) AND a.type <> 'no_submission' AND a.due_at >= $3
             AND i.status = 'published' AND (i.publish_at IS NULL OR i.publish_at <= $3)
             AND m.status = 'published' AND (m.publish_at IS NULL OR m.publish_at <= $3)
             AND (p.id IS NULL
                  OR (p.status = 'published' AND (p.publish_at IS NULL OR p.publish_at <= $3)))
             AND NOT (g.published_score IS NOT NULL OR g.published_feedback IS NOT NULL)
             AND NOT EXISTS (SELECT 1 FROM assessment.submission_index s
                             WHERE s.assignment_id = a.item_id AND s.student_id = $1
                               AND s.status = 'submitted')
           ORDER BY i.course_id, a.due_at"#,
        student_id,
        course_ids,
        now
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| NextDeadline {
            course_id: row.course_id,
            assignment_id: row.assignment_id,
            title: row.title,
            due_at: row.due_at,
        })
        .collect())
}

/// What staff still have to do per course, over the current students: submitted work with no
/// draft score, and draft grades that are not published. Reads the draft columns, so only a staff
/// path may call it.
pub async fn todo_counts(
    conn: &mut PgConnection,
    course_ids: &[Uuid],
) -> Result<Vec<TodoCounts>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT i.course_id AS "course_id!",
                  (SELECT count(*) FROM (
                       SELECT DISTINCT s.assignment_id, s.student_id
                       FROM assessment.submission_index s
                       JOIN content.item_list si ON si.id = s.assignment_id
                       JOIN teaching.effective_members m
                         ON m.course_id = si.course_id AND m.user_id = s.student_id
                        AND m.role = 'student'
                       LEFT JOIN gradebook.grades g
                         ON g.assignment_id = s.assignment_id AND g.student_id = s.student_id
                       WHERE si.course_id = i.course_id AND s.status = 'submitted'
                         AND g.draft_score IS NULL) pending) AS "to_grade!",
                  (SELECT count(*) FROM gradebook.grades g
                   JOIN content.item_list gi ON gi.id = g.assignment_id
                   JOIN teaching.effective_members m
                     ON m.course_id = gi.course_id AND m.user_id = g.student_id
                    AND m.role = 'student'
                   WHERE gi.course_id = i.course_id
                     AND (g.draft_score, g.draft_feedback)
                         IS DISTINCT FROM (g.published_score, g.published_feedback))
                      AS "draft_grades!",
                  EXISTS (SELECT 1 FROM gradebook.grades g
                          JOIN content.assignment_list ga ON ga.item_id = g.assignment_id
                          JOIN content.item_list gi ON gi.id = g.assignment_id
                          WHERE gi.course_id = i.course_id AND ga.type = 'quiz'
                            AND (g.draft_score, g.draft_feedback)
                                IS DISTINCT FROM (g.published_score, g.published_feedback))
                      AS "from_quizzes!"
           FROM (SELECT DISTINCT course_id FROM content.item_list WHERE course_id = ANY($1)) i"#,
        course_ids
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| TodoCounts {
            course_id: row.course_id,
            to_grade: row.to_grade,
            draft_grades: row.draft_grades,
            from_quizzes: row.from_quizzes,
        })
        .collect())
}
