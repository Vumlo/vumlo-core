use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A row of `files.files`.
#[derive(Debug, Clone)]
pub struct File {
    pub id: Uuid,
    pub uploaded_by: Uuid,
    pub name: String,
    pub mime: String,
    pub size: i64,
    pub sha256: String,
    pub created_at: DateTime<Utc>,
}

/// Bytes that `put` wrote to storage and `insert` has not described yet.
#[derive(Debug, Clone)]
pub struct Stored {
    pub id: Uuid,
    pub size: i64,
    pub sha256: String,
    /// The first 512 bytes, for the content-type check of the caller.
    pub head: Vec<u8>,
}
