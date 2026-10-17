//! SQL only. Every function takes the connection it runs on and never begins a transaction.
//!
//! `content`, `teaching` and `identity` tables are read here with plain JOINs and never written.
//! The students of a course come from the view `teaching.effective_members`. Every read that
//! staff can reach selects `status = 'submitted'` rows only (ADR-0011): a draft has no way into it.

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sqlx::PgConnection;
use sqlx::types::Json;
use uuid::Uuid;

use super::model::{Attempt, Draft, LatestAttempt, StudentSubmission};

type Body = Json<Map<String, Value>>;

// ---------------------------------------------------------------------------------------------
// Drafts
// ---------------------------------------------------------------------------------------------

/// Saves the draft of a student: the first save inserts it, later ones replace its content. The
/// last writer wins and nothing is versioned.
pub async fn upsert_draft(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    student_id: Uuid,
    content: &Map<String, Value>,
) -> Result<Draft, sqlx::Error> {
    let row = sqlx::query!(
        r#"INSERT INTO assessment.submissions (id, assignment_id, student_id, status, content)
           VALUES ($1, $2, $3, 'draft', $4)
           ON CONFLICT (assignment_id, student_id) WHERE status = 'draft'
           DO UPDATE SET content = EXCLUDED.content, updated_at = now()
           RETURNING content AS "content: Body", updated_at"#,
        Uuid::now_v7(),
        assignment_id,
        student_id,
        Json(content) as Json<&Map<String, Value>>
    )
    .fetch_one(conn)
    .await?;
    Ok(Draft {
        content: row.content.0,
        updated_at: row.updated_at,
    })
}

pub async fn find_draft(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    student_id: Uuid,
) -> Result<Option<Draft>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT content AS "content: Body", updated_at FROM assessment.submissions
           WHERE assignment_id = $1 AND student_id = $2 AND status = 'draft'"#,
        assignment_id,
        student_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|row| Draft {
        content: row.content.0,
        updated_at: row.updated_at,
    }))
}

pub async fn delete_draft(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    student_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM assessment.submissions
         WHERE assignment_id = $1 AND student_id = $2 AND status = 'draft'",
        assignment_id,
        student_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// A draft that a submit holds.
pub struct LockedDraft {
    pub id: Uuid,
    pub content: Map<String, Value>,
}

/// Locks the draft row `FOR UPDATE`. A second submit of the same draft waits here, and when it
/// is let in the row is no longer a draft, so it finds nothing.
pub async fn lock_draft(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    student_id: Uuid,
) -> Result<Option<LockedDraft>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT id, content AS "content: Body" FROM assessment.submissions
           WHERE assignment_id = $1 AND student_id = $2 AND status = 'draft'
           FOR UPDATE"#,
        assignment_id,
        student_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|row| LockedDraft {
        id: row.id,
        content: row.content.0,
    }))
}

// ---------------------------------------------------------------------------------------------
// Attempts
// ---------------------------------------------------------------------------------------------

/// The number of the student's latest attempt, 0 when there is none. Attempts are numbered
/// without gaps, so it is also how many there are.
pub async fn last_attempt_number(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    student_id: Uuid,
) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT COALESCE(max(attempt), 0) AS "last!" FROM assessment.submissions
           WHERE assignment_id = $1 AND student_id = $2 AND status = 'submitted'"#,
        assignment_id,
        student_id
    )
    .fetch_one(conn)
    .await
}

/// Turns the locked draft into attempt `attempt`, in place: the row is the attempt.
pub async fn submit_draft(
    conn: &mut PgConnection,
    id: Uuid,
    attempt: i32,
    submitted_at: DateTime<Utc>,
    is_late: bool,
) -> Result<Attempt, sqlx::Error> {
    let row = sqlx::query!(
        r#"UPDATE assessment.submissions
           SET status = 'submitted', attempt = $2, submitted_at = $3, is_late = $4,
               updated_at = now()
           WHERE id = $1
           RETURNING id, attempt AS "attempt!", content AS "content: Body",
                     submitted_at AS "submitted_at!", is_late"#,
        id,
        attempt,
        submitted_at,
        is_late
    )
    .fetch_one(conn)
    .await?;
    Ok(Attempt {
        id: row.id,
        attempt: row.attempt,
        content: row.content.0,
        submitted_at: row.submitted_at,
        is_late: row.is_late,
    })
}

/// The submitted attempts of one student, newest first, after the attempt `after` (a row id).
pub async fn list_attempts(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    student_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Attempt>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT id, attempt AS "attempt!", content AS "content: Body",
                  submitted_at AS "submitted_at!", is_late
           FROM assessment.submissions
           WHERE assignment_id = $1 AND student_id = $2 AND status = 'submitted'
             AND ($3::uuid IS NULL
                  OR attempt < (SELECT s.attempt FROM assessment.submissions s WHERE s.id = $3))
           ORDER BY attempt DESC
           LIMIT $4"#,
        assignment_id,
        student_id,
        after,
        limit
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Attempt {
            id: row.id,
            attempt: row.attempt,
            content: row.content.0,
            submitted_at: row.submitted_at,
            is_late: row.is_late,
        })
        .collect())
}

/// Whether staff may open the attempts of this user: a current student of the course, or anyone
/// with a submitted attempt (an exclusion never touches submissions, ADR-0033).
pub async fn is_student_or_has_attempts(
    conn: &mut PgConnection,
    course_id: Uuid,
    assignment_id: Uuid,
    student_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM teaching.effective_members
                          WHERE course_id = $1 AND user_id = $3 AND role = 'student')
               OR EXISTS (SELECT 1 FROM assessment.submissions
                          WHERE assignment_id = $2 AND student_id = $3 AND status = 'submitted')
               AS "found!""#,
        course_id,
        assignment_id,
        student_id
    )
    .fetch_one(conn)
    .await
}

/// The current students of the course after `after`, by surname, name and id, at most `limit`,
/// each with their latest submitted attempt. The cursor is a user id; its place in the order is
/// read from `identity.users`.
pub async fn list_students(
    conn: &mut PgConnection,
    course_id: Uuid,
    assignment_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<StudentSubmission>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT u.id AS student_id, u.name, u.surname,
                  s.id AS "attempt_id?", s.attempt AS "attempt?", s.submitted_at AS "submitted_at?",
                  s.is_late AS "is_late?"
           FROM teaching.effective_members m
           JOIN identity.users u ON u.id = m.user_id
           LEFT JOIN LATERAL (
               SELECT id, attempt, submitted_at, is_late FROM assessment.submissions
               WHERE assignment_id = $2 AND student_id = u.id AND status = 'submitted'
               ORDER BY attempt DESC LIMIT 1
           ) s ON true
           WHERE m.course_id = $1 AND m.role = 'student'
             AND ($3::uuid IS NULL
                  OR (u.surname, u.name, u.id) > (SELECT c.surname, c.name, c.id
                                                  FROM identity.users c WHERE c.id = $3))
           ORDER BY u.surname, u.name, u.id
           LIMIT $4"#,
        course_id,
        assignment_id,
        after,
        limit
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| StudentSubmission {
            student_id: row.student_id,
            name: row.name,
            surname: row.surname,
            latest: match (row.attempt_id, row.attempt, row.submitted_at, row.is_late) {
                (Some(id), Some(attempt), Some(submitted_at), Some(is_late)) => {
                    Some(LatestAttempt {
                        id,
                        attempt,
                        submitted_at,
                        is_late,
                    })
                }
                _ => None,
            },
        })
        .collect())
}

// ---------------------------------------------------------------------------------------------
// Callbacks for content: the has-submitted checks and the purge of drafts
// ---------------------------------------------------------------------------------------------

pub async fn has_submitted_for_assignment(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM assessment.submissions
                          WHERE assignment_id = $1 AND status = 'submitted') AS "has!""#,
        assignment_id
    )
    .fetch_one(conn)
    .await
}

/// The module and its sub-modules.
pub async fn has_submitted_in_module(
    conn: &mut PgConnection,
    module_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM assessment.submissions s
               JOIN content.items i ON i.id = s.assignment_id
               JOIN content.modules m ON m.id = i.module_id
               WHERE (m.id = $1 OR m.parent_id = $1) AND s.status = 'submitted'
           ) AS "has!""#,
        module_id
    )
    .fetch_one(conn)
    .await
}

pub async fn has_submitted_in_course(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM assessment.submissions s
               JOIN content.items i ON i.id = s.assignment_id
               WHERE i.course_id = $1 AND s.status = 'submitted'
           ) AS "has!""#,
        course_id
    )
    .fetch_one(conn)
    .await
}

/// Deletes the drafts of one assignment. Returns how many went.
pub async fn purge_assignment(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM assessment.submissions WHERE assignment_id = $1 AND status = 'draft'",
        assignment_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// Deletes the drafts of a module's and its sub-modules' assignments.
pub async fn purge_module(conn: &mut PgConnection, module_id: Uuid) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM assessment.submissions
         WHERE status = 'draft'
           AND assignment_id IN (SELECT i.id FROM content.items i
                                 JOIN content.modules m ON m.id = i.module_id
                                 WHERE m.id = $1 OR m.parent_id = $1)",
        module_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// Deletes the drafts of every assignment of a course.
pub async fn purge_course(conn: &mut PgConnection, course_id: Uuid) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM assessment.submissions
         WHERE status = 'draft'
           AND assignment_id IN (SELECT id FROM content.items WHERE course_id = $1)",
        course_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// How many sessions of this database wait for a lock of any kind. Tests poll it to know that a
/// writer is blocked behind another one, instead of sleeping and hoping.
///
/// A plain runtime query: it reads a system view, not our schema, so there is nothing for sqlx to
/// check.
#[cfg(test)]
pub async fn lock_waiters(conn: &mut PgConnection) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM pg_stat_activity
         WHERE wait_event_type = 'Lock' AND datname = current_database()",
    )
    .fetch_one(conn)
    .await
}
