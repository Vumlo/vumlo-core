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
/// One file at a time on its own connection, no transaction: `files::delete` removes the object
/// after the row, and a transaction that could still roll back must not wrap that. A file that
/// fails is logged by id and the sweep goes on.
// ponytail: a save naming a 24h-old unreferenced file between the check and the delete leaves a
// dangling reference that reads as 404; lock the file row in `ensure_usable` if it ever matters.
pub async fn sweep(
    db: &PgPool,
    storage: &files::Storage,
    now: DateTime<Utc>,
) -> Result<u64, FileError> {
    let mut conn = db.acquire().await?;
    let candidates = files::candidates(&mut conn, now - GRACE).await?;
    let mut deleted = 0;
    for id in candidates {
        match delete_if_orphan(&mut conn, storage, id).await {
            Ok(true) => deleted += 1,
            Ok(false) => {}
            Err(error) => tracing::warn!(file_id = %id, %error, "sweeping a file failed"),
        }
    }
    Ok(deleted)
}

/// Checks the references right before the delete, so a save that just named the file wins.
async fn delete_if_orphan(
    conn: &mut sqlx::PgConnection,
    storage: &files::Storage,
    id: Uuid,
) -> Result<bool, FileError> {
    if repo::is_referenced(conn, id).await? {
        return Ok(false);
    }
    files::delete(conn, storage, id).await?;
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
