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

use super::model::{Attempt, Draft, LatestAttempt, LatestContent, StudentName, StudentSubmission};

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
    auto_scores: Option<&Value>,
) -> Result<Attempt, sqlx::Error> {
    let row = sqlx::query!(
        r#"UPDATE assessment.submissions
           SET status = 'submitted', attempt = $2, submitted_at = $3, is_late = $4,
               auto_scores = $5, updated_at = now()
           WHERE id = $1
           RETURNING id, attempt AS "attempt!", content AS "content: Body",
                     submitted_at AS "submitted_at!", is_late"#,
        id,
        attempt,
        submitted_at,
        is_late,
        auto_scores.map(Json) as Option<Json<&Value>>
    )
    .fetch_one(conn)
    .await?;
    Ok(Attempt {
        id: row.id,
        attempt: row.attempt,
        content: row.content.0,
        submitted_at: row.submitted_at,
        is_late: row.is_late,
        auto_scores: None,
    })
}

/// The stored per-question scores of attempts, by attempt id: for the staff path only
/// (ADR-0050 point 9). An attempt without scores is left out.
pub async fn auto_scores_of(
    conn: &mut PgConnection,
    attempt_ids: &[Uuid],
) -> Result<Vec<(Uuid, Map<String, Value>)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT id, auto_scores AS "auto_scores!: Body" FROM assessment.submissions
           WHERE id = ANY($1) AND auto_scores IS NOT NULL"#,
        attempt_ids
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.id, row.auto_scores.0))
        .collect())
}

/// The latest attempt of a student with its stored scores, for the result a student reads after
/// their grade is published (ADR-0050 point 8).
pub async fn latest_with_scores(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    student_id: Uuid,
) -> Result<Option<(Attempt, Map<String, Value>)>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT id, attempt AS "attempt!", content AS "content: Body",
                  submitted_at AS "submitted_at!", is_late,
                  auto_scores AS "auto_scores: Body"
           FROM assessment.submissions
           WHERE assignment_id = $1 AND student_id = $2 AND status = 'submitted'
           ORDER BY attempt DESC LIMIT 1"#,
        assignment_id,
        student_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|row| {
        (
            Attempt {
                id: row.id,
                attempt: row.attempt,
                content: row.content.0,
                submitted_at: row.submitted_at,
                is_late: row.is_late,
                auto_scores: None,
            },
            row.auto_scores.map(|scores| scores.0).unwrap_or_default(),
        )
    }))
}

/// The latest submitted attempt of each student who has one, with the student's name: current
/// and former students alike (ADR-0051).
pub async fn latest_contents(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<Vec<LatestContent>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT DISTINCT ON (s.student_id)
                  s.student_id, u.name, u.surname, s.attempt AS "attempt!",
                  s.submitted_at AS "submitted_at!", s.is_late, s.content AS "content: Body"
           FROM assessment.submissions s
           JOIN identity.users u ON u.id = s.student_id
           WHERE s.assignment_id = $1 AND s.status = 'submitted'
           ORDER BY s.student_id, s.attempt DESC"#,
        assignment_id
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| LatestContent {
            student: StudentName {
                user_id: row.student_id,
                name: row.name,
                surname: row.surname,
            },
            attempt: row.attempt,
            submitted_at: row.submitted_at,
            is_late: row.is_late,
            content: row.content.0,
        })
        .collect())
}

/// The current students of the course in roster order (surname, name, id).
pub async fn current_students(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<Vec<StudentName>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT u.id, u.name, u.surname
           FROM teaching.effective_members m
           JOIN identity.users u ON u.id = m.user_id
           WHERE m.course_id = $1 AND m.role = 'student'
           ORDER BY u.surname, u.name, u.id"#,
        course_id
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| StudentName {
            user_id: row.id,
            name: row.name,
            surname: row.surname,
        })
        .collect())
}

/// Which of `ids` are current students of the course.
pub async fn students_among(
    conn: &mut PgConnection,
    course_id: Uuid,
    ids: &[Uuid],
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        // A view's columns are inferred as nullable; `user_id` is not null in its definition.
        r#"SELECT user_id AS "user_id!" FROM teaching.effective_members
           WHERE course_id = $1 AND role = 'student' AND user_id = ANY($2)"#,
        course_id,
        ids
    )
    .fetch_all(conn)
    .await
}

/// Takes the lock that serializes the submits of one form until the transaction ends
/// (ADR-0051 point 4).
pub async fn lock_form(conn: &mut PgConnection, assignment_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "SELECT pg_advisory_xact_lock(hashtextextended('form:' || $1::text, 0))",
        assignment_id.to_string()
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The submitted attempts of one student, newest first, after the attempt `after` (a row id). The
/// cursor counts only if it is an attempt of this student at this assignment: a row id from
/// anywhere else would otherwise tell the caller its attempt number.
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
                  OR attempt < (SELECT s.attempt FROM assessment.submissions s
                                WHERE s.id = $3 AND s.assignment_id = $1 AND s.student_id = $2
                                  AND s.status = 'submitted'))
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
            auto_scores: None,
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

/// The students of the course after `after`, by surname, name and id, at most `limit`, each with
/// their latest submitted attempt: the current students, and the former ones who have a submitted
/// attempt for this assignment (ADR-0037). The cursor is a user id; its place in the order is read
/// from `identity.users`.
pub async fn list_students(
    conn: &mut PgConnection,
    course_id: Uuid,
    assignment_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<StudentSubmission>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"WITH candidates AS (
               SELECT user_id AS "id" FROM teaching.effective_members
               WHERE course_id = $1 AND role = 'student'
             UNION
               SELECT student_id FROM assessment.submissions
               WHERE assignment_id = $2 AND status = 'submitted'
           )
           SELECT u.id AS student_id, u.name, u.surname,
                  NOT EXISTS (SELECT 1 FROM teaching.effective_members m
                              WHERE m.course_id = $1 AND m.user_id = u.id AND m.role = 'student')
                      AS "former!",
                  s.id AS "attempt_id?", s.attempt AS "attempt?", s.submitted_at AS "submitted_at?",
                  s.is_late AS "is_late?"
           FROM candidates c
           JOIN identity.users u ON u.id = c.id
           LEFT JOIN LATERAL (
               SELECT id, attempt, submitted_at, is_late FROM assessment.submissions
               WHERE assignment_id = $2 AND student_id = u.id AND status = 'submitted'
               ORDER BY attempt DESC LIMIT 1
           ) s ON true
           WHERE ($3::uuid IS NULL
                  OR (u.surname, u.name, u.id) > (SELECT x.surname, x.name, x.id
                                                  FROM identity.users x WHERE x.id = $3))
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
            former: row.former,
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
