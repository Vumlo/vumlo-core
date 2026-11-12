//! SQL only. Every function takes the connection it runs on and never begins a transaction.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{File, Stored};

pub async fn insert(
    conn: &mut PgConnection,
    stored: &Stored,
    uploaded_by: Uuid,
    name: &str,
    mime: &str,
) -> Result<File, sqlx::Error> {
    sqlx::query_as!(
        File,
        "INSERT INTO files.files (id, uploaded_by, name, mime, size, sha256)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, uploaded_by, name, mime, size, sha256, created_at",
        stored.id,
        uploaded_by,
        name,
        mime,
        stored.size,
        stored.sha256
    )
    .fetch_one(conn)
    .await
}

pub async fn get(conn: &mut PgConnection, id: Uuid) -> Result<Option<File>, sqlx::Error> {
    sqlx::query_as!(
        File,
        "SELECT id, uploaded_by, name, mime, size, sha256, created_at
         FROM files.files WHERE id = $1",
        id
    )
    .fetch_optional(conn)
    .await
}

/// The rows that exist among `ids`, in ascending id, locked until the transaction ends.
pub async fn lock(conn: &mut PgConnection, ids: &[Uuid]) -> Result<Vec<File>, sqlx::Error> {
    sqlx::query_as!(
        File,
        "SELECT id, uploaded_by, name, mime, size, sha256, created_at
         FROM files.files WHERE id = ANY($1) ORDER BY id FOR UPDATE",
        ids
    )
    .fetch_all(conn)
    .await
}

/// Like [`lock`] for one row, but a row that someone else holds is not waited for.
pub async fn try_lock(conn: &mut PgConnection, id: Uuid) -> Result<Option<File>, sqlx::Error> {
    sqlx::query_as!(
        File,
        "SELECT id, uploaded_by, name, mime, size, sha256, created_at
         FROM files.files WHERE id = $1 FOR UPDATE SKIP LOCKED",
        id
    )
    .fetch_optional(conn)
    .await
}

pub async fn delete(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM files.files WHERE id = $1", id)
        .execute(conn)
        .await?;
    Ok(())
}

pub async fn older_than(
    conn: &mut PgConnection,
    cutoff: DateTime<Utc>,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT id FROM files.files WHERE created_at < $1 ORDER BY created_at",
        cutoff
    )
    .fetch_all(conn)
    .await
}
