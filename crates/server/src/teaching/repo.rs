//! SQL only. Every function takes the connection it runs on and never begins a transaction.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{Course, CourseKind, Membership};
use crate::policy::{CourseRole, CourseStatus};

/// A course together with the asking user's membership in it, if any.
pub struct CourseRow {
    pub course: Course,
    pub role: Option<CourseRole>,
}

/// The shape of every course-with-membership query below.
struct RawCourseRow {
    id: Uuid,
    kind: CourseKind,
    title: String,
    status: CourseStatus,
    owner_id: Uuid,
    created_at: DateTime<Utc>,
    member_role: Option<CourseRole>,
}

impl From<RawCourseRow> for CourseRow {
    fn from(raw: RawCourseRow) -> Self {
        Self {
            course: Course {
                id: raw.id,
                kind: raw.kind,
                title: raw.title,
                status: raw.status,
                owner_id: raw.owner_id,
                created_at: raw.created_at,
            },
            role: raw.member_role,
        }
    }
}

pub async fn insert_course(
    conn: &mut PgConnection,
    id: Uuid,
    title: &str,
    owner_id: Uuid,
) -> Result<Course, sqlx::Error> {
    sqlx::query_as!(
        Course,
        r#"INSERT INTO teaching.courses (id, title, owner_id, created_by)
           VALUES ($1, $2, $3, $3)
           RETURNING id, kind AS "kind: CourseKind", title,
                     status AS "status: CourseStatus", owner_id, created_at"#,
        id,
        title,
        owner_id
    )
    .fetch_one(conn)
    .await
}

pub async fn set_published(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<Course, sqlx::Error> {
    sqlx::query_as!(
        Course,
        r#"UPDATE teaching.courses SET status = 'published', updated_at = now()
           WHERE id = $1
           RETURNING id, kind AS "kind: CourseKind", title,
                     status AS "status: CourseStatus", owner_id, created_at"#,
        course_id
    )
    .fetch_one(conn)
    .await
}

/// The course and `user_id`'s membership in it.
pub async fn find_course(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
) -> Result<Option<CourseRow>, sqlx::Error> {
    let row = sqlx::query_as!(
        RawCourseRow,
        r#"SELECT c.id, c.kind AS "kind: CourseKind", c.title,
                  c.status AS "status: CourseStatus", c.owner_id, c.created_at,
                  m.role AS "member_role?: CourseRole"
           FROM teaching.courses c
           LEFT JOIN teaching.memberships m ON m.course_id = c.id AND m.user_id = $2
           WHERE c.id = $1"#,
        course_id,
        user_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(CourseRow::from))
}

/// Locks the course row until the transaction ends. Writers on one course queue here.
///
/// It returns the course only, never a membership. A statement that waits for a row lock
/// re-reads just the locked row; anything joined to it keeps the statement's earlier snapshot.
/// So the caller reads memberships in a later statement, once it holds the lock, and sees what
/// the writer it waited for committed.
pub async fn lock_course(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<Option<Course>, sqlx::Error> {
    sqlx::query_as!(
        Course,
        r#"SELECT id, kind AS "kind: CourseKind", title,
                  status AS "status: CourseStatus", owner_id, created_at
           FROM teaching.courses
           WHERE id = $1
           FOR UPDATE"#,
        course_id
    )
    .fetch_optional(conn)
    .await
}

/// Courses `user_id` owns or belongs to, after `after`, in id order, at most `limit`. A student
/// membership only counts once the course is published, the same rule `policy` applies per
/// course; it lives here too so a page is not emptied by rows the caller may not see.
pub async fn list_courses_of(
    conn: &mut PgConnection,
    user_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<CourseRow>, sqlx::Error> {
    let rows = sqlx::query_as!(
        RawCourseRow,
        r#"SELECT c.id, c.kind AS "kind: CourseKind", c.title,
                  c.status AS "status: CourseStatus", c.owner_id, c.created_at,
                  m.role AS "member_role?: CourseRole"
           FROM teaching.courses c
           LEFT JOIN teaching.memberships m ON m.course_id = c.id AND m.user_id = $1
           WHERE (m.user_id IS NOT NULL OR c.owner_id = $1)
             AND (c.status = 'published' OR c.owner_id = $1 OR m.role IN ('teacher', 'assistant'))
             AND ($2::uuid IS NULL OR c.id > $2)
           ORDER BY c.id
           LIMIT $3"#,
        user_id,
        after,
        limit
    )
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(CourseRow::from).collect())
}

pub async fn find_member_role(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
) -> Result<Option<CourseRole>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT role AS "role: CourseRole" FROM teaching.memberships
           WHERE course_id = $1 AND user_id = $2"#,
        course_id,
        user_id
    )
    .fetch_optional(conn)
    .await
}

/// Adds the member or changes their role; `created_at` is kept on a change.
pub async fn upsert_membership(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
    role: CourseRole,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO teaching.memberships (course_id, user_id, role) VALUES ($1, $2, $3)
         ON CONFLICT (course_id, user_id) DO UPDATE SET role = EXCLUDED.role",
        course_id,
        user_id,
        role.as_str()
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn delete_membership(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM teaching.memberships WHERE course_id = $1 AND user_id = $2",
        course_id,
        user_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// The members of a course after `after` (a user id), in user-id order, at most `limit`.
pub async fn list_members(
    conn: &mut PgConnection,
    course_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Membership>, sqlx::Error> {
    sqlx::query_as!(
        Membership,
        r#"SELECT m.course_id, m.user_id, m.role AS "role: CourseRole",
                  (m.user_id = c.owner_id) AS "is_owner!"
           FROM teaching.memberships m
           JOIN teaching.courses c ON c.id = m.course_id
           WHERE m.course_id = $1 AND ($2::uuid IS NULL OR m.user_id > $2)
           ORDER BY m.user_id
           LIMIT $3"#,
        course_id,
        after,
        limit
    )
    .fetch_all(conn)
    .await
}
