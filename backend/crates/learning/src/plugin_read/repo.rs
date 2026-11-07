//! SQL only. This flow reads the tables of the four `learning` modules, which share its layer, and
//! the declared views `auth.people` and `teaching.effective_members`. It writes nothing.

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sqlx::PgConnection;
use sqlx::types::Json;
use uuid::Uuid;

use super::model::{Cursor, PluginContact, PluginGrade, PluginMember};

/// A list resumed after `cursor`; `at` is `None` for the first page.
pub struct After {
    at: Option<DateTime<Utc>>,
    a: Uuid,
    b: Uuid,
}

impl From<Option<Cursor>> for After {
    fn from(cursor: Option<Cursor>) -> Self {
        Self {
            at: cursor.map(|c| c.at),
            a: cursor.map_or(Uuid::nil(), |c| c.a),
            b: cursor.map_or(Uuid::nil(), |c| c.b),
        }
    }
}

pub struct AssignmentRow {
    pub id: Uuid,
    pub course_id: Uuid,
    pub kind: String,
    pub title: String,
    pub due_at: Option<DateTime<Utc>>,
    pub max_points: String,
    pub config: Map<String, Value>,
    pub updated_at: DateTime<Utc>,
}

/// The course and the type of an assignment, whether or not a student could see it yet.
pub async fn assignment_header(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<Option<(Uuid, String)>, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT i.course_id, a.type AS kind FROM content.items i
         JOIN content.assignments a ON a.item_id = i.id
         WHERE i.id = $1",
        assignment_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|row| (row.course_id, row.kind)))
}

/// The assignments a student could see now: the course is out of draft, and the module, its parent
/// and the item are published and not scheduled later. Their title is the published one. Sorted by
/// the last change. `only` limits the courses (`None`: every course), `one` limits to a single
/// assignment.
pub async fn assignments(
    conn: &mut PgConnection,
    only: Option<&[Uuid]>,
    one: Option<Uuid>,
    since: Option<DateTime<Utc>>,
    after: &After,
    limit: i64,
) -> Result<Vec<AssignmentRow>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT id AS "id!", course_id AS "course_id!", kind AS "kind!", title AS "title!",
                  due_at, max_points AS "max_points!", config AS "config!: Json<Map<String, Value>>",
                  updated AS "updated_at!"
           FROM (
               SELECT i.id, i.course_id, a.type AS kind, i.published_title AS title, a.due_at,
                      a.max_points::text AS max_points, a.config,
                      GREATEST(i.updated_at, a.updated_at) AS updated
               FROM content.items i
               JOIN content.assignments a ON a.item_id = i.id
               JOIN content.modules m ON m.id = i.module_id
               LEFT JOIN content.modules p ON p.id = m.parent_id
               JOIN teaching.courses c ON c.id = i.course_id
               WHERE i.status = 'published' AND (i.publish_at IS NULL OR i.publish_at <= now())
                 AND m.status = 'published' AND (m.publish_at IS NULL OR m.publish_at <= now())
                 AND (p.id IS NULL
                      OR (p.status = 'published' AND (p.publish_at IS NULL OR p.publish_at <= now())))
                 AND c.status <> 'draft'
                 AND ($1::uuid[] IS NULL OR i.course_id = ANY($1))
                 AND ($2::uuid IS NULL OR i.id = $2)
           ) t
           WHERE ($3::timestamptz IS NULL OR updated >= $3)
             AND ($4::timestamptz IS NULL OR (updated, id) > ($4, $5))
           ORDER BY updated, id
           LIMIT $6"#,
        only,
        one,
        since,
        after.at,
        after.a,
        limit
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| AssignmentRow {
            id: row.id,
            course_id: row.course_id,
            kind: row.kind,
            title: row.title,
            due_at: row.due_at,
            max_points: row.max_points,
            config: row.config.0,
            updated_at: row.updated_at,
        })
        .collect())
}

pub struct SubmittedRow {
    pub id: Uuid,
    pub course_id: Uuid,
    pub assignment_id: Uuid,
    pub assignment_type: String,
    pub student_id: Uuid,
    pub attempt: i32,
    pub content: Map<String, Value>,
    pub submitted_at: DateTime<Utc>,
}

/// A submitted attempt by its row id. A draft has no way through `status = 'submitted'`.
pub async fn submitted(
    conn: &mut PgConnection,
    submission_id: Uuid,
) -> Result<Option<SubmittedRow>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT s.id, i.course_id, s.assignment_id, a.type AS assignment_type, s.student_id,
                  s.attempt AS "attempt!", s.content AS "content: Json<Map<String, Value>>",
                  s.submitted_at AS "submitted_at!"
           FROM assessment.submissions s
           JOIN content.items i ON i.id = s.assignment_id
           JOIN content.assignments a ON a.item_id = s.assignment_id
           WHERE s.id = $1 AND s.status = 'submitted'"#,
        submission_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|row| SubmittedRow {
        id: row.id,
        course_id: row.course_id,
        assignment_id: row.assignment_id,
        assignment_type: row.assignment_type,
        student_id: row.student_id,
        attempt: row.attempt,
        content: row.content.0,
        submitted_at: row.submitted_at,
    }))
}

/// The course of a submitted attempt.
pub async fn submission_course(
    conn: &mut PgConnection,
    submission_id: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT i.course_id FROM assessment.submissions s
         JOIN content.items i ON i.id = s.assignment_id
         WHERE s.id = $1 AND s.status = 'submitted'",
        submission_id
    )
    .fetch_optional(conn)
    .await
}

pub struct SubmissionListRow {
    pub id: Uuid,
    pub student_id: Uuid,
    pub attempt: i32,
    pub submitted_at: DateTime<Utc>,
    pub is_late: bool,
    pub graded_at: Option<DateTime<Utc>>,
}

/// The submitted attempts of an assignment, by the time they were submitted.
pub async fn submissions(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    since: Option<DateTime<Utc>>,
    after: &After,
    limit: i64,
) -> Result<Vec<SubmissionListRow>, sqlx::Error> {
    sqlx::query_as!(
        SubmissionListRow,
        r#"SELECT s.id, s.student_id, s.attempt AS "attempt!", s.submitted_at AS "submitted_at!",
                  s.is_late AS "is_late!", g.published_at AS graded_at
           FROM assessment.submissions s
           LEFT JOIN gradebook.grades g
                  ON g.assignment_id = s.assignment_id AND g.student_id = s.student_id
           WHERE s.assignment_id = $1 AND s.status = 'submitted'
             AND ($2::timestamptz IS NULL OR s.submitted_at >= $2)
             AND ($3::timestamptz IS NULL OR (s.submitted_at, s.id) > ($3, $4))
           ORDER BY s.submitted_at, s.id
           LIMIT $5"#,
        assignment_id,
        since,
        after.at,
        after.a,
        limit
    )
    .fetch_all(conn)
    .await
}

pub struct AttemptRow {
    pub id: Uuid,
    pub student_id: Uuid,
    pub attempt: i32,
    pub scores: Option<Map<String, Value>>,
    pub started_at: DateTime<Utc>,
    pub submitted_at: DateTime<Utc>,
}

/// The submitted attempts of a quiz with the score per question, by the time they were submitted.
pub async fn attempts(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    since: Option<DateTime<Utc>>,
    after: &After,
    limit: i64,
) -> Result<Vec<AttemptRow>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT id, student_id, attempt AS "attempt!",
                  auto_scores AS "auto_scores: Json<Map<String, Value>>",
                  created_at, submitted_at AS "submitted_at!"
           FROM assessment.submissions
           WHERE assignment_id = $1 AND status = 'submitted'
             AND ($2::timestamptz IS NULL OR submitted_at >= $2)
             AND ($3::timestamptz IS NULL OR (submitted_at, id) > ($3, $4))
           ORDER BY submitted_at, id
           LIMIT $5"#,
        assignment_id,
        since,
        after.at,
        after.a,
        limit
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| AttemptRow {
            id: row.id,
            student_id: row.student_id,
            attempt: row.attempt,
            scores: row.auto_scores.map(|scores| scores.0),
            started_at: row.created_at,
            submitted_at: row.submitted_at,
        })
        .collect())
}

/// The published grades of the courses in `only` (`None`: every course), by the time they were
/// published. Never the draft columns.
pub async fn grades(
    conn: &mut PgConnection,
    only: Option<&[Uuid]>,
    assignment_id: Option<Uuid>,
    since: Option<DateTime<Utc>>,
    after: &After,
    limit: i64,
) -> Result<Vec<PluginGrade>, sqlx::Error> {
    sqlx::query_as!(
        PluginGrade,
        r#"SELECT i.course_id, g.assignment_id, g.student_id, g.published_score::text AS score,
                  a.max_points::text AS "max_points!", g.published_by,
                  g.published_at AS "published_at!"
           FROM gradebook.grades g
           JOIN content.items i ON i.id = g.assignment_id
           JOIN content.assignments a ON a.item_id = g.assignment_id
           WHERE g.published_at IS NOT NULL
             AND ($1::uuid[] IS NULL OR i.course_id = ANY($1))
             AND ($2::uuid IS NULL OR g.assignment_id = $2)
             AND ($3::timestamptz IS NULL OR g.published_at >= $3)
             AND ($4::timestamptz IS NULL OR (g.published_at, g.assignment_id, g.student_id) > ($4, $5, $6))
           ORDER BY g.published_at, g.assignment_id, g.student_id
           LIMIT $7"#,
        only,
        assignment_id,
        since,
        after.at,
        after.a,
        after.b,
        limit
    )
    .fetch_all(conn)
    .await
}

/// The people of a course, by user id: `role` limits them to one course role.
pub async fn members(
    conn: &mut PgConnection,
    course_id: Uuid,
    role: Option<&str>,
    after: &After,
    limit: i64,
) -> Result<Vec<PluginMember>, sqlx::Error> {
    sqlx::query_as!(
        PluginMember,
        r#"SELECT m.user_id AS "user_id!", p.name AS "name!", p.surname AS "surname!",
                  m.role AS "role!", m.source AS "source!"
           FROM teaching.effective_members m
           JOIN auth.people p ON p.id = m.user_id
           WHERE m.course_id = $1
             AND ($2::text IS NULL OR m.role = $2)
             AND ($3::timestamptz IS NULL OR m.user_id > $4)
           ORDER BY m.user_id
           LIMIT $5"#,
        course_id,
        role,
        after.at,
        after.a,
        limit
    )
    .fetch_all(conn)
    .await
}

/// The email of those of `ids` who are in a course of `only` (`None`: any course).
pub async fn contacts(
    conn: &mut PgConnection,
    ids: &[Uuid],
    only: Option<&[Uuid]>,
) -> Result<Vec<PluginContact>, sqlx::Error> {
    sqlx::query_as!(
        PluginContact,
        r#"SELECT p.id AS "user_id!", p.email AS "email!"
           FROM auth.people p
           WHERE p.id = ANY($1) AND p.email IS NOT NULL
             AND EXISTS (SELECT 1 FROM teaching.effective_members m
                         WHERE m.user_id = p.id AND ($2::uuid[] IS NULL OR m.course_id = ANY($2)))
           ORDER BY p.id"#,
        ids,
        only
    )
    .fetch_all(conn)
    .await
}
