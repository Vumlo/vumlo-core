//! SQL only. Every function takes the connection it runs on and never begins a transaction.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{
    Course, CourseGroupLink, CourseKind, Exclusion, MemberSource, Membership, RosterEntry,
};
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
    term_id: Option<Uuid>,
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
                term_id: raw.term_id,
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
    term_id: Option<Uuid>,
) -> Result<Course, sqlx::Error> {
    sqlx::query_as!(
        Course,
        r#"INSERT INTO teaching.courses (id, title, owner_id, created_by, term_id)
           VALUES ($1, $2, $3, $3, $4)
           RETURNING id, kind AS "kind: CourseKind", title,
                     status AS "status: CourseStatus", owner_id, term_id, created_at"#,
        id,
        title,
        owner_id,
        term_id
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
                     status AS "status: CourseStatus", owner_id, term_id, created_at"#,
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
                  c.status AS "status: CourseStatus", c.owner_id, c.term_id, c.created_at,
                  m.role AS "member_role?: CourseRole"
           FROM teaching.courses c
           LEFT JOIN teaching.effective_members m ON m.course_id = c.id AND m.user_id = $2
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
                  status AS "status: CourseStatus", owner_id, term_id, created_at
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
                  c.status AS "status: CourseStatus", c.owner_id, c.term_id, c.created_at,
                  m.role AS "member_role?: CourseRole"
           FROM teaching.courses c
           LEFT JOIN teaching.effective_members m ON m.course_id = c.id AND m.user_id = $1
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

/// The user's place on the roster of the course, from the view: a direct membership, or a student
/// through a linked group (ADR-0033).
pub async fn find_member(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
) -> Result<Option<RosterEntry>, sqlx::Error> {
    sqlx::query_as!(
        RosterEntry,
        r#"SELECT role AS "role!: CourseRole", source AS "source!: MemberSource"
           FROM teaching.effective_members
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

/// The roster of a course after `after` (a user id), in user-id order, at most `limit`: direct
/// members and students through linked groups.
pub async fn list_members(
    conn: &mut PgConnection,
    course_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Membership>, sqlx::Error> {
    sqlx::query_as!(
        Membership,
        r#"SELECT m.course_id AS "course_id!", m.user_id AS "user_id!",
                  m.role AS "role!: CourseRole", (m.user_id = c.owner_id) AS "is_owner!",
                  m.source AS "source!: MemberSource"
           FROM teaching.effective_members m
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

/// Changes the title and/or the term. `set_term` says whether `term_id` was sent (it may be null).
pub async fn update_course(
    conn: &mut PgConnection,
    course_id: Uuid,
    title: Option<&str>,
    set_term: bool,
    term_id: Option<Uuid>,
) -> Result<Course, sqlx::Error> {
    sqlx::query_as!(
        Course,
        r#"UPDATE teaching.courses
           SET title = COALESCE($2, title),
               term_id = CASE WHEN $3 THEN $4 ELSE term_id END,
               updated_at = now()
           WHERE id = $1
           RETURNING id, kind AS "kind: CourseKind", title,
                     status AS "status: CourseStatus", owner_id, term_id, created_at"#,
        course_id,
        title,
        set_term,
        term_id
    )
    .fetch_one(conn)
    .await
}

// ---------------------------------------------------------------------------------------------
// Group links and exclusions (ADR-0033)
// ---------------------------------------------------------------------------------------------

/// Links the course to the group or subgroup. `None` when that link exists already.
pub async fn insert_link(
    conn: &mut PgConnection,
    id: Uuid,
    course_id: Uuid,
    group_id: Uuid,
    subgroup_id: Option<Uuid>,
    created_by: Uuid,
) -> Result<Option<CourseGroupLink>, sqlx::Error> {
    sqlx::query_as!(
        CourseGroupLink,
        "INSERT INTO teaching.course_groups (id, course_id, group_id, subgroup_id, created_by)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT DO NOTHING
         RETURNING id, course_id, group_id, subgroup_id, created_at",
        id,
        course_id,
        group_id,
        subgroup_id,
        created_by
    )
    .fetch_optional(conn)
    .await
}

pub async fn find_link_by_target(
    conn: &mut PgConnection,
    course_id: Uuid,
    group_id: Uuid,
    subgroup_id: Option<Uuid>,
) -> Result<Option<CourseGroupLink>, sqlx::Error> {
    sqlx::query_as!(
        CourseGroupLink,
        "SELECT id, course_id, group_id, subgroup_id, created_at
         FROM teaching.course_groups
         WHERE course_id = $1 AND group_id = $2 AND subgroup_id IS NOT DISTINCT FROM $3",
        course_id,
        group_id,
        subgroup_id
    )
    .fetch_optional(conn)
    .await
}

pub async fn list_links(
    conn: &mut PgConnection,
    course_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<CourseGroupLink>, sqlx::Error> {
    sqlx::query_as!(
        CourseGroupLink,
        "SELECT id, course_id, group_id, subgroup_id, created_at
         FROM teaching.course_groups
         WHERE course_id = $1 AND ($2::uuid IS NULL OR id > $2)
         ORDER BY id LIMIT $3",
        course_id,
        after,
        limit
    )
    .fetch_all(conn)
    .await
}

pub async fn delete_link(
    conn: &mut PgConnection,
    course_id: Uuid,
    link_id: Uuid,
) -> Result<Option<CourseGroupLink>, sqlx::Error> {
    sqlx::query_as!(
        CourseGroupLink,
        "DELETE FROM teaching.course_groups WHERE course_id = $1 AND id = $2
         RETURNING id, course_id, group_id, subgroup_id, created_at",
        course_id,
        link_id
    )
    .fetch_optional(conn)
    .await
}

/// Records that the user is off the roster of this course. Keeps an existing exclusion as it is.
pub async fn insert_exclusion(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
    created_by: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO teaching.course_exclusions (course_id, user_id, created_by)
         VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        course_id,
        user_id,
        created_by
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn delete_exclusion(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM teaching.course_exclusions WHERE course_id = $1 AND user_id = $2",
        course_id,
        user_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// The exclusions of a course after `after` (a user id), in user-id order, at most `limit`.
pub async fn list_exclusions(
    conn: &mut PgConnection,
    course_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Exclusion>, sqlx::Error> {
    sqlx::query_as!(
        Exclusion,
        "SELECT user_id, created_by, created_at FROM teaching.course_exclusions
         WHERE course_id = $1 AND ($2::uuid IS NULL OR user_id > $2)
         ORDER BY user_id LIMIT $3",
        course_id,
        after,
        limit
    )
    .fetch_all(conn)
    .await
}

/// Would a linked group put the user on the roster, ignoring a direct membership and exclusions?
pub async fn in_group_roster(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM teaching.group_roster
                          WHERE course_id = $1 AND user_id = $2) AS "found!""#,
        course_id,
        user_id
    )
    .fetch_one(conn)
    .await
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
