//! SQL of the work read models: what a student sees of their own standing and what staff still
//! have to grade (ADR-0049, ADR-0052). The standing selects published grade columns only.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::content::{AssignmentType, BuiltinType};
use crate::gradebook::model::{StaffFacts, WorkFacts, WorkStatus};

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
        r#"SELECT a.item_id AS assignment_id, i.course_id,
                  a.type AS "assignment_type: BuiltinType",
                  COALESCE(a.due_at IS NOT NULL
                           AND CASE WHEN a.accept_late THEN a.late_until ELSE a.due_at END < $3,
                           false) AS "closed!",
                  (SELECT s.updated_at FROM assessment.submissions s
                   WHERE s.assignment_id = a.item_id AND s.student_id = $1
                     AND s.status = 'draft') AS draft_saved_at,
                  (SELECT count(*) FROM assessment.submissions s
                   WHERE s.assignment_id = a.item_id AND s.student_id = $1
                     AND s.status = 'submitted') AS "attempts!",
                  g.published_score,
                  (g.published_score IS NOT NULL OR g.published_feedback IS NOT NULL)
                      AS "graded!",
                  g.published_at
           FROM content.assignments a
           JOIN content.items i ON i.id = a.item_id
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
        r#"SELECT a.item_id AS assignment_id,
                  (SELECT count(*) FROM teaching.effective_members m
                   WHERE m.course_id = $1 AND m.role = 'student') AS "graded_total!",
                  (SELECT count(DISTINCT s.student_id) FROM assessment.submissions s
                   JOIN teaching.effective_members m
                     ON m.course_id = $1 AND m.user_id = s.student_id AND m.role = 'student'
                   WHERE s.assignment_id = a.item_id AND s.status = 'submitted') AS "submitted!",
                  (SELECT count(DISTINCT s.student_id) FROM assessment.submissions s
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
           FROM content.assignments a
           JOIN content.items i ON i.id = a.item_id
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
