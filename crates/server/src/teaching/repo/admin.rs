//! The read models behind the administrator's screens (ADR-0053). Read-only joins across schemas.

use sqlx::PgConnection;
use uuid::Uuid;

use crate::policy::CourseStatus;
use crate::teaching::model::{AdminCourseRow, CourseLineRow, LinkedCourseRow};

/// Regular courses after `after` (a course id), by id, with owner, term and student count.
pub async fn admin_courses(
    conn: &mut PgConnection,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<AdminCourseRow>, sqlx::Error> {
    sqlx::query_as!(
        AdminCourseRow,
        r#"SELECT c.id AS course_id, c.code, c.title, c.status AS "status: CourseStatus",
                  c.owner_id, o.name AS owner_name, o.surname AS owner_surname,
                  c.term_id, t.name AS "term_name?",
                  (SELECT count(*) FROM teaching.effective_members m
                   WHERE m.course_id = c.id AND m.role = 'student') AS "student_count!"
           FROM teaching.courses c
           JOIN identity.users o ON o.id = c.owner_id
           LEFT JOIN org.terms t ON t.id = c.term_id
           WHERE c.kind = 'regular' AND ($1::uuid IS NULL OR c.id > $1)
           ORDER BY c.id
           LIMIT $2"#,
        after,
        limit
    )
    .fetch_all(conn)
    .await
}

/// The regular courses that use a group or one of its subgroups, once each, by code.
pub async fn linked_courses_of_group(
    conn: &mut PgConnection,
    group_id: Uuid,
) -> Result<Vec<LinkedCourseRow>, sqlx::Error> {
    sqlx::query_as!(
        LinkedCourseRow,
        r#"SELECT c.id AS course_id, c.code, c.title, o.name AS owner_name,
                  o.surname AS owner_surname, min(l.created_at) AS "linked_on!"
           FROM teaching.course_groups l
           JOIN teaching.courses c ON c.id = l.course_id
           JOIN identity.users o ON o.id = c.owner_id
           WHERE l.group_id = $1 AND c.kind = 'regular'
           GROUP BY c.id, c.code, c.title, o.name, o.surname
           ORDER BY c.code, c.title, c.id"#,
        group_id
    )
    .fetch_all(conn)
    .await
}

/// The regular courses a person owns.
pub async fn owned_courses(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<Vec<CourseLineRow>, sqlx::Error> {
    sqlx::query_as!(
        CourseLineRow,
        "SELECT id AS course_id, code, title FROM teaching.courses
         WHERE owner_id = $1 AND kind = 'regular' ORDER BY code, title, id",
        user_id
    )
    .fetch_all(conn)
    .await
}

/// The regular courses where a person is a teacher or an assistant and not the owner.
pub async fn taught_courses(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<Vec<CourseLineRow>, sqlx::Error> {
    sqlx::query_as!(
        CourseLineRow,
        r#"SELECT c.id AS course_id, c.code, c.title
           FROM teaching.effective_members m
           JOIN teaching.courses c ON c.id = m.course_id
           WHERE m.user_id = $1 AND m.role IN ('teacher', 'assistant')
             AND c.kind = 'regular' AND c.owner_id <> $1
           ORDER BY c.code, c.title, c.id"#,
        user_id
    )
    .fetch_all(conn)
    .await
}

/// How many published regular courses a person studies in now.
pub async fn studied_courses(conn: &mut PgConnection, user_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM teaching.effective_members m
           JOIN teaching.courses c ON c.id = m.course_id
           WHERE m.user_id = $1 AND m.role = 'student' AND c.kind = 'regular'
             AND c.status = 'published'"#,
        user_id
    )
    .fetch_one(conn)
    .await
}
