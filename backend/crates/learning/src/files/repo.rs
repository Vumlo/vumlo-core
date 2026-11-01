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

/// Whether any item or submission names the file.
pub async fn is_referenced(conn: &mut PgConnection, file_id: Uuid) -> Result<bool, sqlx::Error> {
    let referenced = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM content.item_files WHERE file_id = $1)
               OR EXISTS (SELECT 1 FROM assessment.submission_files WHERE file_id = $1) AS "referenced!""#,
        file_id
    )
    .fetch_one(conn)
    .await?;
    Ok(referenced)
}

/// How many of the submission rows `held_by` belong to `student` on `assignment_id`. Equal to
/// `held_by.len()` when every one of them is an attempt or draft of that author on that assignment.
pub async fn count_own_rows_on(
    conn: &mut PgConnection,
    held_by: &[Uuid],
    assignment_id: Uuid,
    student: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM assessment.submissions
            WHERE id = ANY($1) AND assignment_id = $2 AND student_id = $3"#,
        held_by,
        assignment_id,
        student
    )
    .fetch_one(conn)
    .await
}
