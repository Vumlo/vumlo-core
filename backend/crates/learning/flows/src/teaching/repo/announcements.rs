//! SQL for the announcements of a course (ADR-0076).

use sqlx::PgConnection;
use uuid::Uuid;

use crate::teaching::model::Announcement;

pub async fn insert(
    conn: &mut PgConnection,
    id: Uuid,
    course_id: Uuid,
    author_id: Uuid,
    title: &str,
    body: &str,
) -> Result<Announcement, sqlx::Error> {
    sqlx::query_as!(
        Announcement,
        "INSERT INTO teaching.announcements (id, course_id, author_id, title, body)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, course_id, author_id, title, body, created_at, updated_at",
        id,
        course_id,
        author_id,
        title,
        body
    )
    .fetch_one(conn)
    .await
}

pub async fn find(conn: &mut PgConnection, id: Uuid) -> Result<Option<Announcement>, sqlx::Error> {
    sqlx::query_as!(
        Announcement,
        "SELECT id, course_id, author_id, title, body, created_at, updated_at
         FROM teaching.announcements WHERE id = $1",
        id
    )
    .fetch_optional(conn)
    .await
}

/// A course's announcements, newest first, after `after` (the id of the last one seen).
pub async fn list(
    conn: &mut PgConnection,
    course_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Announcement>, sqlx::Error> {
    sqlx::query_as!(
        Announcement,
        "SELECT id, course_id, author_id, title, body, created_at, updated_at
         FROM teaching.announcements
         WHERE course_id = $1 AND ($2::uuid IS NULL OR id < $2)
         ORDER BY id DESC LIMIT $3",
        course_id,
        after,
        limit
    )
    .fetch_all(conn)
    .await
}

pub async fn update(
    conn: &mut PgConnection,
    id: Uuid,
    title: Option<&str>,
    body: Option<&str>,
) -> Result<Option<Announcement>, sqlx::Error> {
    sqlx::query_as!(
        Announcement,
        "UPDATE teaching.announcements
         SET title = COALESCE($2, title), body = COALESCE($3, body), updated_at = now()
         WHERE id = $1
         RETURNING id, course_id, author_id, title, body, created_at, updated_at",
        id,
        title,
        body
    )
    .fetch_optional(conn)
    .await
}

pub async fn delete(conn: &mut PgConnection, id: Uuid) -> Result<u64, sqlx::Error> {
    let done = sqlx::query!("DELETE FROM teaching.announcements WHERE id = $1", id)
        .execute(conn)
        .await?;
    Ok(done.rows_affected())
}
