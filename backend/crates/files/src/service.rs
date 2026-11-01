use std::ops::Range;

use chrono::{DateTime, Utc};
use object_store::{GetOptions, GetRange, GetResult, ObjectStoreExt, PutPayload};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{File, Stored};
use super::repo;
use super::storage::{Storage, key};

const HEAD_LEN: usize = 512;

#[derive(Debug, thiserror::Error)]
pub enum FilesError {
    #[error("the file is empty")]
    Empty,
    #[error("the file is larger than the allowed size")]
    TooLarge,
    #[error("files.s3.endpoint {0}")]
    Config(String),
    #[error(transparent)]
    Storage(#[from] object_store::Error),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Writes `data` to storage under a key derived from `id`. No transaction: the caller inserts the
/// description afterwards, and if that fails it must call [`delete`] itself: the orphan sweep walks
/// rows, so an object without a row is never found again (ADR-0069).
// ponytail: buffered upload, stream it when an ADR adds a stream crate
pub async fn put<B: AsRef<[u8]> + Into<PutPayload>>(
    storage: &Storage,
    id: Uuid,
    data: B,
    max_size: u64,
) -> Result<Stored, FilesError> {
    let bytes = data.as_ref();
    if bytes.is_empty() {
        return Err(FilesError::Empty);
    }
    if bytes.len() as u64 > max_size {
        return Err(FilesError::TooLarge);
    }
    let stored = Stored {
        id,
        size: bytes.len() as i64,
        sha256: hex(&Sha256::digest(bytes)),
        head: bytes[..bytes.len().min(HEAD_LEN)].to_vec(),
    };
    storage.store().put(&key(id), data.into()).await?;
    Ok(stored)
}

/// Describes bytes that `put` stored, on the caller's connection.
pub async fn insert(
    conn: &mut PgConnection,
    stored: &Stored,
    uploaded_by: Uuid,
    name: &str,
    mime: &str,
) -> Result<File, FilesError> {
    Ok(repo::insert(conn, stored, uploaded_by, name, mime).await?)
}

pub async fn get(conn: &mut PgConnection, id: Uuid) -> Result<Option<File>, FilesError> {
    Ok(repo::get(conn, id).await?)
}

/// The bytes of `file`, or only `range` of them. A range past the end is a storage error.
pub async fn read(
    storage: &Storage,
    file: &File,
    range: Option<Range<u64>>,
) -> Result<GetResult, FilesError> {
    let options = GetOptions {
        range: range.map(GetRange::Bounded),
        ..GetOptions::default()
    };
    Ok(storage.store().get_opts(&key(file.id), options).await?)
}

/// Removes the row, then the object. A failed object delete is logged and ignored: the row is
/// the truth, and an object without a row is harmless.
pub async fn delete(
    conn: &mut PgConnection,
    storage: &Storage,
    id: Uuid,
) -> Result<(), FilesError> {
    repo::delete(conn, id).await?;
    if let Err(error) = storage.store().delete(&key(id)).await {
        tracing::warn!(file_id = %id, %error, "deleting the stored object failed");
    }
    Ok(())
}

/// Ids of the files created before `older_than`. The caller (in `learning`) drops those that an
/// owner still references and deletes the rest.
// ponytail: loads every file older than 24 h, referenced ones included (the sweep then checks each
// id); page it or anti-join the reference tables once the table is large.
pub async fn candidates(
    conn: &mut PgConnection,
    older_than: DateTime<Utc>,
) -> Result<Vec<Uuid>, FilesError> {
    Ok(repo::older_than(conn, older_than).await?)
}

#[cfg(test)]
mod tests;
