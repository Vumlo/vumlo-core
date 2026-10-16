//! SQL only. Every function takes the connection it runs on and never begins a transaction.
//!
//! `content`, `teaching` and `identity` tables are read here with plain JOINs and never written
//! (ADR-0028). Student-facing reads select `published_*` columns only.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use sqlx::types::Decimal;
use uuid::Uuid;

use super::model::{Cell, Column, ColumnStatus, MyGrade};
use crate::policy::{CourseRole, CourseStatus, Release};

/// An assignment with the course it belongs to and the release state of every row on the path to
/// it, which `policy` needs to decide whether a student may see it.
pub struct AssignmentContext {
    pub course_id: Uuid,
    pub item: Release,
    /// The module the item is in.
    pub module: Release,
    /// That module's parent, when the module is a sub-module.
    pub parent: Option<Release>,
}

/// A course with the asking user's membership, which is all `policy` needs of it.
pub struct CourseRow {
    pub owner_id: Uuid,
    pub status: CourseStatus,
    pub role: Option<CourseRole>,
}

pub async fn find_assignment(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<Option<AssignmentContext>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT i.course_id,
                  i.status = 'published' AS "item_published!", i.publish_at AS item_publish_at,
                  m.status = 'published' AS "module_published!", m.publish_at AS module_publish_at,
                  p.status = 'published' AS "parent_published?", p.publish_at AS parent_publish_at
           FROM content.assignments a
           JOIN content.items i ON i.id = a.item_id
           JOIN content.modules m ON m.id = i.module_id
           LEFT JOIN content.modules p ON p.id = m.parent_id
           WHERE a.item_id = $1"#,
        assignment_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|row| AssignmentContext {
        course_id: row.course_id,
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
    }))
}

/// The course and `user_id`'s membership in it, read here because `gradebook` calls no function
/// of `teaching` (ADR-0028).
pub async fn find_course(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
) -> Result<Option<CourseRow>, sqlx::Error> {
    sqlx::query_as!(
        CourseRow,
        r#"SELECT c.owner_id, c.status AS "status: CourseStatus", m.role AS "role?: CourseRole"
           FROM teaching.courses c
           LEFT JOIN teaching.memberships m ON m.course_id = c.id AND m.user_id = $2
           WHERE c.id = $1"#,
        course_id,
        user_id
    )
    .fetch_optional(conn)
    .await
}

/// Takes a shared lock on the assignment row until the transaction ends and returns `max_points`
/// as it is in that row. A change of `max_points` and a delete take the row `FOR UPDATE`, so they
/// wait for this transaction and a grade written here is seen by their later checks. When this
/// statement itself had to wait, it returns the value the writer committed. It selects from
/// `content.assignments` alone: a waiting statement re-reads only the locked row. A lock, not a
/// write. `None`: the assignment is gone.
pub async fn lock_assignment_shared(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<Option<Decimal>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT max_points FROM content.assignments WHERE item_id = $1 FOR SHARE",
        assignment_id
    )
    .fetch_optional(conn)
    .await
}

pub async fn is_student_member(
    conn: &mut PgConnection,
    course_id: Uuid,
    student_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM teaching.memberships
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
        "SELECT user_id FROM teaching.memberships
         WHERE course_id = $1 AND role = 'student' AND user_id = ANY($2)",
        course_id,
        student_ids
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

/// Appends one history row. `score` and `feedback` are the values after the action.
pub async fn insert_history(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    student_id: Uuid,
    action: &str,
    actor_id: Uuid,
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
           JOIN teaching.memberships m
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
}

/// The current student members of the course after `after`, by surname, name and id, at most
/// `limit`. The cursor is a user id; its place in the order is read from `identity.users`, so it
/// still works for a student who left.
pub async fn students(
    conn: &mut PgConnection,
    course_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<StudentRow>, sqlx::Error> {
    sqlx::query_as!(
        StudentRow,
        r#"SELECT u.id AS user_id, u.name, u.surname
           FROM teaching.memberships m
           JOIN identity.users u ON u.id = m.user_id
           WHERE m.course_id = $1 AND m.role = 'student'
             AND ($2::uuid IS NULL
                  OR (u.surname, u.name, u.id) > (SELECT c.surname, c.name, c.id
                                                  FROM identity.users c WHERE c.id = $2))
           ORDER BY u.surname, u.name, u.id
           LIMIT $3"#,
        course_id,
        after,
        limit
    )
    .fetch_all(conn)
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
        r#"SELECT i.id AS assignment_id, i.title, a.max_points, a.is_bonus,
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

/// How many sessions of this database wait for a lock of any kind. Tests poll it to know that a
/// writer is blocked behind another one, instead of sleeping and hoping.
///
/// A runtime query with a constant string: `cargo sqlx prepare` does not cover test-only code, so
/// a `query!` here would be missing from `.sqlx/` (an exception to AGENTS §8).
#[cfg(test)]
pub async fn lock_waiters(conn: &mut PgConnection) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM pg_stat_activity
         WHERE wait_event_type = 'Lock' AND datname = current_database()",
    )
    .fetch_one(conn)
    .await
}
