//! The read models behind `GET /me/course-cards` (ADR-0052). They join `content`, `assessment` and
//! `gradebook` read-only, as every read model does (ADR-0015). A student's queries read published
//! values and published rows only; the staff queries read the draft columns.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use super::{CourseRow, RawCourseRow};
use crate::policy::{CourseRole, CourseStatus};
use crate::teaching::model::{
    CardCourse, CourseKind, DraftItem, NextDeadline, Progress, TodoCounts,
};

/// The regular courses of the user, with what a card shows of the course itself. A student sees
/// a course only while it is published or archived (ADR-0048).
pub async fn card_courses(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<Vec<CardCourse>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT c.id, c.kind AS "kind: CourseKind", c.title, c.status AS "status: CourseStatus",
                  c.owner_id, c.term_id, c.created_at, c.code, c.description,
                  t.name AS "term_name?", t.ends_on AS "term_ends?",
                  o.name AS owner_name, o.surname AS owner_surname,
                  m.role AS "member_role?: CourseRole",
                  (SELECT count(*) FROM teaching.effective_members x
                   WHERE x.course_id = c.id AND x.role = 'student') AS "student_count!",
                  (SELECT count(*) FROM teaching.effective_members x
                   WHERE x.course_id = c.id AND x.role = 'assistant') AS "assistant_count!"
           FROM (SELECT course_id AS id FROM teaching.effective_members WHERE user_id = $1
                 UNION
                 SELECT id FROM teaching.courses WHERE owner_id = $1) mine
           JOIN teaching.courses c ON c.id = mine.id
           LEFT JOIN teaching.effective_members m ON m.course_id = c.id AND m.user_id = $1
           LEFT JOIN org.terms t ON t.id = c.term_id
           JOIN identity.users o ON o.id = c.owner_id
           WHERE c.kind = 'regular'
             AND (c.status IN ('published', 'archived') OR c.owner_id = $1
                  OR m.role IN ('teacher', 'assistant'))
           ORDER BY c.title, c.id"#,
        user_id
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let raw = RawCourseRow {
                id: row.id,
                kind: row.kind,
                title: row.title,
                status: row.status,
                owner_id: row.owner_id,
                term_id: row.term_id,
                created_at: row.created_at,
                member_role: row.member_role,
            };
            CardCourse {
                course: CourseRow::from(raw).course,
                code: row.code,
                description: row.description,
                term_name: row.term_name,
                term_ends: row.term_ends,
                owner_name: row.owner_name,
                owner_surname: row.owner_surname,
                role: row.member_role,
                student_count: row.student_count,
                assistant_count: row.assistant_count,
            }
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
                      EXISTS (SELECT 1 FROM assessment.submissions s
                              WHERE s.assignment_id = a.item_id AND s.student_id = $1
                                AND s.status = 'submitted') AS attempted
               FROM content.assignments a
               JOIN content.items i ON i.id = a.item_id
               JOIN content.modules m ON m.id = i.module_id
               LEFT JOIN content.modules p ON p.id = m.parent_id
               LEFT JOIN gradebook.grades g ON g.assignment_id = a.item_id AND g.student_id = $1
               WHERE i.course_id = ANY($2) AND a.type <> 'no_submission'
                 AND i.status = 'published' AND (i.publish_at IS NULL OR i.publish_at <= $3)
                 AND m.status = 'published' AND (m.publish_at IS NULL OR m.publish_at <= $3)
                 AND (p.id IS NULL
                      OR (p.status = 'published' AND (p.publish_at IS NULL OR p.publish_at <= $3))))
           SELECT course_id, count(*) AS "total!",
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
        r#"SELECT DISTINCT ON (i.course_id) i.course_id, a.item_id AS assignment_id,
                  i.published_title AS "title!", a.due_at AS "due_at!"
           FROM content.assignments a
           JOIN content.items i ON i.id = a.item_id
           JOIN content.modules m ON m.id = i.module_id
           LEFT JOIN content.modules p ON p.id = m.parent_id
           LEFT JOIN gradebook.grades g ON g.assignment_id = a.item_id AND g.student_id = $1
           WHERE i.course_id = ANY($2) AND a.type <> 'no_submission' AND a.due_at >= $3
             AND i.status = 'published' AND (i.publish_at IS NULL OR i.publish_at <= $3)
             AND m.status = 'published' AND (m.publish_at IS NULL OR m.publish_at <= $3)
             AND (p.id IS NULL
                  OR (p.status = 'published' AND (p.publish_at IS NULL OR p.publish_at <= $3)))
             AND NOT (g.published_score IS NOT NULL OR g.published_feedback IS NOT NULL)
             AND NOT EXISTS (SELECT 1 FROM assessment.submissions s
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
/// draft score, and draft grades that are not published.
pub async fn staff_todo_counts(
    conn: &mut PgConnection,
    course_ids: &[Uuid],
) -> Result<Vec<TodoCounts>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT i.course_id,
                  (SELECT count(*) FROM (
                       SELECT DISTINCT s.assignment_id, s.student_id
                       FROM assessment.submissions s
                       JOIN content.items si ON si.id = s.assignment_id
                       JOIN teaching.effective_members m
                         ON m.course_id = si.course_id AND m.user_id = s.student_id
                        AND m.role = 'student'
                       LEFT JOIN gradebook.grades g
                         ON g.assignment_id = s.assignment_id AND g.student_id = s.student_id
                       WHERE si.course_id = i.course_id AND s.status = 'submitted'
                         AND g.draft_score IS NULL) pending) AS "to_grade!",
                  (SELECT count(*) FROM gradebook.grades g
                   JOIN content.items gi ON gi.id = g.assignment_id
                   JOIN teaching.effective_members m
                     ON m.course_id = gi.course_id AND m.user_id = g.student_id
                    AND m.role = 'student'
                   WHERE gi.course_id = i.course_id
                     AND (g.draft_score, g.draft_feedback)
                         IS DISTINCT FROM (g.published_score, g.published_feedback))
                      AS "draft_grades!",
                  EXISTS (SELECT 1 FROM gradebook.grades g
                          JOIN content.assignments ga ON ga.item_id = g.assignment_id
                          JOIN content.items gi ON gi.id = g.assignment_id
                          WHERE gi.course_id = i.course_id AND ga.type = 'quiz'
                            AND (g.draft_score, g.draft_feedback)
                                IS DISTINCT FROM (g.published_score, g.published_feedback))
                      AS "from_quizzes!"
           FROM (SELECT DISTINCT course_id FROM content.items WHERE course_id = ANY($1)) i"#,
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

/// Items that are drafts inside published modules, newest change first.
pub async fn unpublished_items(
    conn: &mut PgConnection,
    course_ids: &[Uuid],
) -> Result<Vec<DraftItem>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT i.course_id, i.title
           FROM content.items i
           JOIN content.modules m ON m.id = i.module_id
           LEFT JOIN content.modules p ON p.id = m.parent_id
           WHERE i.course_id = ANY($1) AND i.status = 'draft' AND m.status = 'published'
             AND (p.id IS NULL OR p.status = 'published')
           ORDER BY i.updated_at DESC, i.id"#,
        course_ids
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| DraftItem {
            course_id: row.course_id,
            title: row.title,
        })
        .collect())
}

/// The earliest deadline from `now` on, per course, of any assignment.
pub async fn staff_next_deadline(
    conn: &mut PgConnection,
    course_ids: &[Uuid],
    now: DateTime<Utc>,
) -> Result<Vec<NextDeadline>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT DISTINCT ON (i.course_id) i.course_id, a.item_id AS assignment_id,
                  i.title, a.due_at AS "due_at!"
           FROM content.assignments a
           JOIN content.items i ON i.id = a.item_id
           WHERE i.course_id = ANY($1) AND a.due_at >= $2
           ORDER BY i.course_id, a.due_at"#,
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
