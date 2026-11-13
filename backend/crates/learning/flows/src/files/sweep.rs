//! The orphan sweep (ADR-0069): a file that no item and no submission names, and that is more than
//! a day old, is deleted. `files` sits below this crate and cannot read the reference tables, so
//! the sweep is a flow here.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::repo;
use super::service::FileError;

/// A file stays this long after its upload, so that one being attached to a save is not swept.
const GRACE: TimeDelta = TimeDelta::hours(24);
const INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Deletes the files created before `now - 24h` that nothing references and returns how many.
/// One file at a time in its own short transaction. A save that names a file locks its row first
/// (`check`), so the sweep takes the same lock without waiting: a file that is being attached is
/// skipped, and the next round finds it referenced or an orphan. The stored object is removed
/// only after the row is gone for good. A file that fails is logged by id and the sweep goes on.
pub async fn sweep(
    db: &PgPool,
    storage: &files::Storage,
    now: DateTime<Utc>,
) -> Result<u64, FileError> {
    let mut conn = db.acquire().await?;
    let candidates = files::candidates(&mut conn, now - GRACE).await?;
    drop(conn);
    let mut deleted = 0;
    for id in candidates {
        match delete_if_orphan(db, storage, id).await {
            Ok(true) => deleted += 1,
            Ok(false) => {}
            Err(error) => tracing::warn!(file_id = %id, %error, "sweeping a file failed"),
        }
    }
    Ok(deleted)
}

/// Locks the row, checks the references under the lock, and removes the row. A save that names
/// the file afterwards had to lock it first (`files::check`), and finds it gone.
// ponytail: `copy_course_content` copies references without locking the files, so a file whose
// only holder is deleted while a copy of it is still uncommitted can be swept under the copy;
// lock the copied file ids there (`files::lock`) if a copy of a day-old, otherwise unused file
// ever matters.
async fn delete_if_orphan(
    db: &PgPool,
    storage: &files::Storage,
    id: Uuid,
) -> Result<bool, FileError> {
    let mut tx = db.begin().await?;
    if files::try_lock(&mut tx, id).await?.is_none() || repo::is_referenced(&mut tx, id).await? {
        return Ok(false);
    }
    files::forget(&mut tx, id).await?;
    tx.commit().await?;
    files::drop_object(storage, id).await;
    Ok(true)
}

/// Sweeps once an hour until the process ends. Never returns and never panics: a failed round is
/// logged and the next one runs on time.
pub async fn run_sweeper(db: PgPool, storage: files::Storage) {
    loop {
        match sweep(&db, &storage, Utc::now()).await {
            Ok(0) => {}
            Ok(deleted) => tracing::info!(deleted, "swept orphaned files"),
            Err(error) => tracing::error!(%error, "the file sweep failed a round"),
        }
        tokio::time::sleep(INTERVAL).await;
    }
}
