//! SQL only. Reads the reference tables of `content` and `assessment`; `learning` sits on their
//! layer. The rows are written by those modules.

use sqlx::PgConnection;
use uuid::Uuid;

use super::model::ItemReference;

/// The items that name the file, one row per copy.
pub async fn item_references(
    conn: &mut PgConnection,
    file_id: Uuid,
) -> Result<Vec<ItemReference>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT item_id, published FROM content.item_files WHERE file_id = $1",
        file_id
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| ItemReference {
            item_id: row.item_id,
            published: row.published,
        })
        .collect())
}

/// The submissions (drafts and attempts) that name the file.
pub async fn submission_references(
    conn: &mut PgConnection,
    file_id: Uuid,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT submission_id FROM assessment.submission_files WHERE file_id = $1",
        file_id
    )
    .fetch_all(conn)
    .await
}
