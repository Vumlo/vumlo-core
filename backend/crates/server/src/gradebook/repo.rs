//! SQL only. Every function takes the connection it runs on and never begins a transaction.
//!
//! `content`, `teaching` and `identity` tables are read here with plain JOINs and never written
//! (ADR-0028). The students of a course come from the view `teaching.effective_members`. Student-facing reads select `published_*` columns only.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use sqlx::types::Decimal;
use uuid::Uuid;

use super::model::{
    Cell, Column, ColumnStatus, DeadlineRow, HistoryActor, HistoryEntry, MyGrade, PublishedGrade,
    ToGradeRow, UnpublishedContentRow, UnpublishedRow,
};
use crate::policy::Release;

pub async fn is_student_member(
    conn: &mut PgConnection,
    course_id: Uuid,
    student_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM teaching.effective_members
                          WHERE course_id = $1 AND user_id = $2 AND role = 'student') AS "member!""#,
        course_id,
        student_id
    )
    .fetch_one(conn)
    .await
}

/// Which of `student_ids` are students of the course.
pub async fn student_members_among(
    conn: &mut PgConnection,
    course_id: Uuid,
    student_ids: &[Uuid],
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        // A view's columns are inferred as nullable; `user_id` is not null in its definition.
        r#"SELECT user_id AS "user_id!" FROM teaching.effective_members
           WHERE course_id = $1 AND role = 'student' AND user_id = ANY($2)"#,
        course_id,
        student_ids
    )
    .fetch_all(conn)
    .await
}

/// The most rows the teacher's "to grade" list shows (ADR-0052).
pub const TO_GRADE_LIMIT: i64 = 200;

/// The latest attempt of each current student that has no draft score yet, oldest first.
pub async fn to_grade(
    conn: &mut PgConnection,
    course_ids: &[Uuid],
) -> Result<Vec<ToGradeRow>, sqlx::Error> {
    sqlx::query_as!(
        ToGradeRow,
        r#"SELECT s.student_id, u.name AS student_name, u.surname AS student_surname,
                  s.assignment_id, i.title AS assignment_title, i.course_id,
                  s.submitted_at AS "submitted_at!", s.attempt AS "attempt!", a.max_attempts
           FROM assessment.submissions s
           JOIN content.items i ON i.id = s.assignment_id
           JOIN content.assignments a ON a.item_id = i.id
           JOIN teaching.effective_members m
             ON m.course_id = i.course_id AND m.user_id = s.student_id AND m.role = 'student'
           JOIN identity.users u ON u.id = s.student_id
           LEFT JOIN gradebook.grades g
             ON g.assignment_id = s.assignment_id AND g.student_id = s.student_id
           WHERE i.course_id = ANY($1) AND s.status = 'submitted' AND g.draft_score IS NULL
             AND s.attempt = (SELECT max(x.attempt) FROM assessment.submissions x
                              WHERE x.assignment_id = s.assignment_id
                                AND x.student_id = s.student_id AND x.status = 'submitted')
           ORDER BY s.submitted_at, s.id
           LIMIT $2"#,
        course_ids,
        TO_GRADE_LIMIT
    )
    .fetch_all(conn)
    .await
}

/// Assignments with draft grades of current students that are not published.
pub async fn unpublished_grades(
    conn: &mut PgConnection,
    course_ids: &[Uuid],
) -> Result<Vec<UnpublishedRow>, sqlx::Error> {
    sqlx::query_as!(
        UnpublishedRow,
        r#"SELECT g.assignment_id, i.title AS assignment_title, i.course_id,
                  count(*) AS "draft_count!", (a.type = 'quiz') AS "auto_scored!"
           FROM gradebook.grades g
           JOIN content.items i ON i.id = g.assignment_id
           JOIN content.assignments a ON a.item_id = i.id
           JOIN teaching.effective_members m
             ON m.course_id = i.course_id AND m.user_id = g.student_id AND m.role = 'student'
           WHERE i.course_id = ANY($1)
             AND (g.draft_score, g.draft_feedback)
                 IS DISTINCT FROM (g.published_score, g.published_feedback)
           GROUP BY g.assignment_id, i.title, i.course_id, a.type
           ORDER BY i.course_id, i.title, g.assignment_id"#,
        course_ids
    )
    .fetch_all(conn)
    .await
}

/// Assignments due at or after `now`, soonest first, with how many current students submitted.
pub async fn deadlines(
    conn: &mut PgConnection,
    course_ids: &[Uuid],
    now: DateTime<Utc>,
) -> Result<Vec<DeadlineRow>, sqlx::Error> {
    sqlx::query_as!(
        DeadlineRow,
        r#"SELECT a.item_id AS assignment_id, i.title, i.course_id, a.due_at AS "due_at!",
                  (SELECT count(DISTINCT s.student_id) FROM assessment.submissions s
                   JOIN teaching.effective_members m
                     ON m.course_id = i.course_id AND m.user_id = s.student_id
                    AND m.role = 'student'
                   WHERE s.assignment_id = a.item_id AND s.status = 'submitted') AS "submitted!",
                  (SELECT count(*) FROM teaching.effective_members m
                   WHERE m.course_id = i.course_id AND m.role = 'student') AS "total!",
                  (i.status = 'draft') AS "not_open!"
           FROM content.assignments a
           JOIN content.items i ON i.id = a.item_id
           WHERE i.course_id = ANY($1) AND a.due_at >= $2
           ORDER BY a.due_at, i.id"#,
        course_ids,
        now
    )
    .fetch_all(conn)
    .await
}

/// Draft items inside published modules, and published items with edits students do not see.
pub async fn unpublished_content(
    conn: &mut PgConnection,
    course_ids: &[Uuid],
) -> Result<Vec<UnpublishedContentRow>, sqlx::Error> {
    sqlx::query_as!(
        UnpublishedContentRow,
        r#"SELECT i.id AS item_id, i.title, i.course_id,
                  (i.status = 'published') AS "edits!"
           FROM content.items i
           JOIN content.modules m ON m.id = i.module_id
           LEFT JOIN content.modules p ON p.id = m.parent_id
           WHERE i.course_id = ANY($1)
             AND ((i.status = 'draft' AND m.status = 'published'
                   AND (p.id IS NULL OR p.status = 'published'))
                  OR (i.status = 'published'
                      AND (i.title IS DISTINCT FROM i.published_title
                           OR i.body IS DISTINCT FROM i.published_body)))
           ORDER BY i.course_id, i.updated_at DESC, i.id"#,
        course_ids
    )
    .fetch_all(conn)
    .await
}

/// Replaces the draft slot of one cell. The last writer wins.
pub async fn upsert_draft(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    student_id: Uuid,
    score: Option<Decimal>,
    feedback: Option<&str>,
    actor_id: Uuid,
) -> Result<Cell, sqlx::Error> {
    sqlx::query_as!(
        Cell,
        r#"INSERT INTO gradebook.grades
               (assignment_id, student_id, draft_score, draft_feedback, draft_updated_by)
           VALUES ($1, $2, $3, $4, $5)
           ON CONFLICT (assignment_id, student_id) DO UPDATE
           SET draft_score = EXCLUDED.draft_score,
               draft_feedback = EXCLUDED.draft_feedback,
               draft_updated_by = EXCLUDED.draft_updated_by,
               draft_updated_at = now()
           RETURNING draft_score, draft_feedback, published_score, published_feedback,
                     ROW(draft_score, draft_feedback)
                         IS DISTINCT FROM ROW(published_score, published_feedback)
                         AS "has_unpublished_changes!""#,
        assignment_id,
        student_id,
        score,
        feedback,
        actor_id
    )
    .fetch_one(conn)
    .await
}

/// Replaces the draft score of one cell with the score a quiz earned and leaves the draft feedback
/// alone; no person made the change (ADR-0050 point 5). Returns the draft feedback, which the
/// history row repeats.
pub async fn upsert_auto_score(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    student_id: Uuid,
    score: Decimal,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"INSERT INTO gradebook.grades (assignment_id, student_id, draft_score, draft_updated_by)
           VALUES ($1, $2, $3, NULL)
           ON CONFLICT (assignment_id, student_id) DO UPDATE
           SET draft_score = EXCLUDED.draft_score,
               draft_updated_by = NULL,
               draft_updated_at = now()
           RETURNING draft_feedback"#,
        assignment_id,
        student_id,
        score
    )
    .fetch_one(conn)
    .await
}

/// The published value of one cell, if a score or feedback was ever published. Selects
/// `published_*` only (AGENTS section 6).
pub async fn find_published(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    student_id: Uuid,
) -> Result<Option<PublishedGrade>, sqlx::Error> {
    sqlx::query_as!(
        PublishedGrade,
        r#"SELECT published_score AS score, published_feedback AS feedback,
                  published_at AS "published_at!"
           FROM gradebook.grades
           WHERE assignment_id = $1 AND student_id = $2 AND published_at IS NOT NULL"#,
        assignment_id,
        student_id
    )
    .fetch_optional(conn)
    .await
}

/// Appends one history row. `score` and `feedback` are the values after the action. `actor_id`
/// is `None` only for `auto_score`.
pub async fn insert_history(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    student_id: Uuid,
    action: &str,
    actor_id: Option<Uuid>,
    score: Option<Decimal>,
    feedback: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO gradebook.grade_history
             (id, assignment_id, student_id, action, actor_id, score, feedback)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        Uuid::now_v7(),
        assignment_id,
        student_id,
        action,
        actor_id,
        score,
        feedback
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The recorded changes of one cell, newest first: ids below `before`, at most `limit`. Staff
/// read it, so the people who made the changes are named.
pub async fn list_history(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    student_id: Uuid,
    before: Option<Uuid>,
    limit: i64,
) -> Result<Vec<HistoryEntry>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT h.id, h.action, h.score, h.feedback, h.at,
                  h.actor_id AS "actor_id?", u.name AS "actor_name?", u.surname AS "actor_surname?"
           FROM gradebook.grade_history h
           LEFT JOIN identity.users u ON u.id = h.actor_id
           WHERE h.assignment_id = $1 AND h.student_id = $2 AND ($3::uuid IS NULL OR h.id < $3)
           ORDER BY h.id DESC
           LIMIT $4"#,
        assignment_id,
        student_id,
        before,
        limit
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| HistoryEntry {
            id: row.id,
            action: row.action,
            score: row.score,
            feedback: row.feedback,
            actor: row.actor_id.zip(row.actor_name).zip(row.actor_surname).map(
                |((user_id, name), surname)| HistoryActor {
                    user_id,
                    name,
                    surname,
                },
            ),
            at: row.at,
        })
        .collect())
}

/// Locks, in student-id order, the rows of current student members whose draft differs from the
/// published value, and returns their students. `only` limits them to a listed set. Locking in
/// one order keeps two publishes of overlapping sets from deadlocking; a draft save that holds a
/// row makes this wait, and the row is re-checked afterwards.
pub async fn lock_publishable(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    course_id: Uuid,
    only: Option<&[Uuid]>,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT g.student_id
           FROM gradebook.grades g
           JOIN teaching.effective_members m
             ON m.course_id = $2 AND m.user_id = g.student_id AND m.role = 'student'
           WHERE g.assignment_id = $1
             AND ($3::uuid[] IS NULL OR g.student_id = ANY($3))
             AND ROW(g.draft_score, g.draft_feedback)
                 IS DISTINCT FROM ROW(g.published_score, g.published_feedback)
           ORDER BY g.student_id
           FOR UPDATE OF g"#,
        assignment_id,
        course_id,
        only
    )
    .fetch_all(conn)
    .await
}

/// A row that publishing replaced, with what is now published.
pub struct Published {
    pub student_id: Uuid,
    pub score: Option<Decimal>,
    pub feedback: Option<String>,
}

/// Copies draft to published for the locked students whose draft still differs, in a later
/// statement than the lock so that a draft saved while this waited is the one copied.
pub async fn publish_rows(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    student_ids: &[Uuid],
    actor_id: Uuid,
) -> Result<Vec<Published>, sqlx::Error> {
    sqlx::query_as!(
        Published,
        r#"UPDATE gradebook.grades
           SET published_score = draft_score, published_feedback = draft_feedback,
               published_by = $3, published_at = now()
           WHERE assignment_id = $1 AND student_id = ANY($2)
             AND ROW(draft_score, draft_feedback)
                 IS DISTINCT FROM ROW(published_score, published_feedback)
           RETURNING student_id, published_score AS score, published_feedback AS feedback"#,
        assignment_id,
        student_ids,
        actor_id
    )
    .fetch_all(conn)
    .await
}

// ---------------------------------------------------------------------------------------------
// Read models
// ---------------------------------------------------------------------------------------------

/// Every assignment of the course in tree order: top-level modules by position, each followed by
/// its sub-modules, the items of one module by position. The order of `content::repo::tree`.
pub async fn columns(conn: &mut PgConnection, course_id: Uuid) -> Result<Vec<Column>, sqlx::Error> {
    sqlx::query_as!(
        Column,
        r#"SELECT i.id AS assignment_id, i.title, a.max_points, a.is_bonus,
                  i.status AS "status: ColumnStatus"
           FROM content.assignments a
           JOIN content.items i ON i.id = a.item_id
           JOIN content.modules m ON m.id = i.module_id
           LEFT JOIN content.modules p ON p.id = m.parent_id
           WHERE i.course_id = $1
           ORDER BY COALESCE(p.position, m.position), m.parent_id NULLS FIRST, m.position,
                    i.position"#,
        course_id
    )
    .fetch_all(conn)
    .await
}

pub struct StudentRow {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
    /// Not on the roster now, but has grades in the course (ADR-0037).
    pub former: bool,
}

/// The students of the course after `after`, by surname, name and id, at most `limit`: the current
/// students, and the former ones who still have a grade with a value in the course. The cursor is
/// a user id; its place in the order is read from `identity.users`, so it still works for a
/// student who left.
pub async fn students(
    conn: &mut PgConnection,
    course_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<StudentRow>, sqlx::Error> {
    sqlx::query_as!(
        StudentRow,
        r#"WITH candidates AS (
               SELECT user_id AS "id" FROM teaching.effective_members
               WHERE course_id = $1 AND role = 'student'
             UNION
               SELECT g.student_id FROM gradebook.grades g
               JOIN content.items i ON i.id = g.assignment_id
               WHERE i.course_id = $1
                 AND (g.draft_score IS NOT NULL OR g.draft_feedback IS NOT NULL
                      OR g.published_score IS NOT NULL OR g.published_feedback IS NOT NULL)
           )
           SELECT u.id AS user_id, u.name, u.surname,
                  NOT EXISTS (SELECT 1 FROM teaching.effective_members m
                              WHERE m.course_id = $1 AND m.user_id = u.id AND m.role = 'student')
                      AS "former!"
           FROM candidates c
           JOIN identity.users u ON u.id = c.id
           WHERE ($2::uuid IS NULL
                  OR (u.surname, u.name, u.id) > (SELECT x.surname, x.name, x.id
                                                  FROM identity.users x WHERE x.id = $2))
           ORDER BY u.surname, u.name, u.id
           LIMIT $3"#,
        course_id,
        after,
        limit
    )
    .fetch_all(conn)
    .await
}

/// Does the user have a grade with a value in an assignment of the course? Then they are a former
/// student when they are not on the roster.
pub async fn has_grades_of_student(
    conn: &mut PgConnection,
    course_id: Uuid,
    student_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM gradebook.grades g
               JOIN content.items i ON i.id = g.assignment_id
               WHERE i.course_id = $1 AND g.student_id = $2
                 AND (g.draft_score IS NOT NULL OR g.draft_feedback IS NOT NULL
                      OR g.published_score IS NOT NULL OR g.published_feedback IS NOT NULL)
           ) AS "has!""#,
        course_id,
        student_id
    )
    .fetch_one(conn)
    .await
}

/// Does the user have a published grade in the course that they may be shown? (Visibility of the
/// rows is decided later, row by row.)
pub async fn has_published_grades(
    conn: &mut PgConnection,
    course_id: Uuid,
    student_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM gradebook.grades g
               JOIN content.items i ON i.id = g.assignment_id
               WHERE i.course_id = $1 AND g.student_id = $2
                 AND (g.published_score IS NOT NULL OR g.published_feedback IS NOT NULL)
           ) AS "has!""#,
        course_id,
        student_id
    )
    .fetch_one(conn)
    .await
}

pub struct CellRow {
    pub assignment_id: Uuid,
    pub student_id: Uuid,
    pub cell: Cell,
}

/// The grade rows of these students for the assignments of the course.
pub async fn cells(
    conn: &mut PgConnection,
    course_id: Uuid,
    student_ids: &[Uuid],
) -> Result<Vec<CellRow>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT g.assignment_id, g.student_id,
                  g.draft_score, g.draft_feedback, g.published_score, g.published_feedback,
                  ROW(g.draft_score, g.draft_feedback)
                      IS DISTINCT FROM ROW(g.published_score, g.published_feedback)
                      AS "has_unpublished_changes!"
           FROM gradebook.grades g
           JOIN content.items i ON i.id = g.assignment_id
           WHERE i.course_id = $1 AND g.student_id = ANY($2)"#,
        course_id,
        student_ids
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| CellRow {
            assignment_id: row.assignment_id,
            student_id: row.student_id,
            cell: Cell {
                draft_score: row.draft_score,
                draft_feedback: row.draft_feedback,
                published_score: row.published_score,
                published_feedback: row.published_feedback,
                has_unpublished_changes: row.has_unpublished_changes,
            },
        })
        .collect())
}

/// A published grade with the release state of the rows on the path to its assignment.
pub struct MyGradeRow {
    pub grade: MyGrade,
    pub item: Release,
    pub module: Release,
    pub parent: Option<Release>,
}

/// The published grades of one student in one course, in tree order. Only `published_*` columns
/// are selected, and a row appears only when its published score or feedback is set. `open_at`
/// keeps what a student may see at that moment (ADR-0010); `policy` checks every row again.
pub async fn my_published_grades(
    conn: &mut PgConnection,
    course_id: Uuid,
    student_id: Uuid,
    open_at: DateTime<Utc>,
) -> Result<Vec<MyGradeRow>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT i.id AS assignment_id, i.published_title AS "title!", a.max_points, a.is_bonus,
                  g.published_score, g.published_feedback, g.published_at AS "published_at!",
                  i.status = 'published' AS "item_published!", i.publish_at AS item_publish_at,
                  m.status = 'published' AS "module_published!", m.publish_at AS module_publish_at,
                  p.status = 'published' AS "parent_published?", p.publish_at AS parent_publish_at
           FROM gradebook.grades g
           JOIN content.assignments a ON a.item_id = g.assignment_id
           JOIN content.items i ON i.id = a.item_id
           JOIN content.modules m ON m.id = i.module_id
           LEFT JOIN content.modules p ON p.id = m.parent_id
           WHERE g.student_id = $2 AND i.course_id = $1
             AND (g.published_score IS NOT NULL OR g.published_feedback IS NOT NULL)
             AND i.status = 'published' AND (i.publish_at IS NULL OR i.publish_at <= $3)
             AND m.status = 'published' AND (m.publish_at IS NULL OR m.publish_at <= $3)
             AND (p.id IS NULL
                  OR (p.status = 'published' AND (p.publish_at IS NULL OR p.publish_at <= $3)))
           ORDER BY COALESCE(p.position, m.position), m.parent_id NULLS FIRST, m.position,
                    i.position"#,
        course_id,
        student_id,
        open_at
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| MyGradeRow {
            grade: MyGrade {
                assignment_id: row.assignment_id,
                title: row.title,
                max_points: row.max_points,
                is_bonus: row.is_bonus,
                score: row.published_score,
                feedback: row.published_feedback,
                published_at: row.published_at,
            },
            item: Release {
                published: row.item_published,
                publish_at: row.item_publish_at,
            },
            module: Release {
                published: row.module_published,
                publish_at: row.module_publish_at,
            },
            parent: row.parent_published.map(|published| Release {
                published,
                publish_at: row.parent_publish_at,
            }),
        })
        .collect())
}

// ---------------------------------------------------------------------------------------------
// Callbacks for content: the highest score, has-grades checks and the purge
// ---------------------------------------------------------------------------------------------

/// The highest draft or published score of the assignment, `None` when there is none.
pub async fn max_score(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<Option<Decimal>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT GREATEST(max(draft_score), max(published_score)) AS "max"
           FROM gradebook.grades WHERE assignment_id = $1"#,
        assignment_id
    )
    .fetch_one(conn)
    .await
}

pub async fn has_grades_for_assignment(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM gradebook.grades
               WHERE assignment_id = $1
                 AND (draft_score IS NOT NULL OR draft_feedback IS NOT NULL
                      OR published_score IS NOT NULL OR published_feedback IS NOT NULL)
           ) AS "has!""#,
        assignment_id
    )
    .fetch_one(conn)
    .await
}

/// The module and its sub-modules.
pub async fn has_grades_in_module(
    conn: &mut PgConnection,
    module_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM gradebook.grades g
               JOIN content.items i ON i.id = g.assignment_id
               JOIN content.modules m ON m.id = i.module_id
               WHERE (m.id = $1 OR m.parent_id = $1)
                 AND (g.draft_score IS NOT NULL OR g.draft_feedback IS NOT NULL
                      OR g.published_score IS NOT NULL OR g.published_feedback IS NOT NULL)
           ) AS "has!""#,
        module_id
    )
    .fetch_one(conn)
    .await
}

pub async fn has_grades_in_course(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM gradebook.grades g
               JOIN content.items i ON i.id = g.assignment_id
               WHERE i.course_id = $1
                 AND (g.draft_score IS NOT NULL OR g.draft_feedback IS NOT NULL
                      OR g.published_score IS NOT NULL OR g.published_feedback IS NOT NULL)
           ) AS "has!""#,
        course_id
    )
    .fetch_one(conn)
    .await
}

/// Deletes the history and the grade rows of one assignment. Returns how many grade rows went.
pub async fn purge_assignment(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<u64, sqlx::Error> {
    sqlx::query!(
        "DELETE FROM gradebook.grade_history WHERE assignment_id = $1",
        assignment_id
    )
    .execute(&mut *conn)
    .await?;
    let result = sqlx::query!(
        "DELETE FROM gradebook.grades WHERE assignment_id = $1",
        assignment_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// Deletes the history and the grade rows of a module and its sub-modules.
pub async fn purge_module(conn: &mut PgConnection, module_id: Uuid) -> Result<u64, sqlx::Error> {
    sqlx::query!(
        "DELETE FROM gradebook.grade_history
         WHERE assignment_id IN (SELECT i.id FROM content.items i
                                 JOIN content.modules m ON m.id = i.module_id
                                 WHERE m.id = $1 OR m.parent_id = $1)",
        module_id
    )
    .execute(&mut *conn)
    .await?;
    let result = sqlx::query!(
        "DELETE FROM gradebook.grades
         WHERE assignment_id IN (SELECT i.id FROM content.items i
                                 JOIN content.modules m ON m.id = i.module_id
                                 WHERE m.id = $1 OR m.parent_id = $1)",
        module_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// Deletes the history and the grade rows of every assignment of a course.
pub async fn purge_course(conn: &mut PgConnection, course_id: Uuid) -> Result<u64, sqlx::Error> {
    sqlx::query!(
        "DELETE FROM gradebook.grade_history
         WHERE assignment_id IN (SELECT id FROM content.items WHERE course_id = $1)",
        course_id
    )
    .execute(&mut *conn)
    .await?;
    let result = sqlx::query!(
        "DELETE FROM gradebook.grades
         WHERE assignment_id IN (SELECT id FROM content.items WHERE course_id = $1)",
        course_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}
