//! The offline store (ADR-0060): one SQLite file in the app data directory. `cache` holds the last good
//! answer of every `GET`, `outbox` holds the writes that wait for a connection, `meta` holds who the
//! store belongs to. Calls are short and made under a mutex; nothing here is async.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};

pub struct CacheEntry {
    pub body: String,
    pub etag: Option<String>,
    /// Unix milliseconds of the last time the server confirmed this answer.
    pub fetched_at: i64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct OutboxRow {
    pub id: i64,
    pub method: String,
    pub path: String,
    pub body: Option<String>,
    pub idem_key: String,
    pub created_at: i64,
    pub attempts: i64,
    /// Unix seconds before which the row is not tried again.
    pub next_at: i64,
    /// `pending`, `sending` or `failed`.
    pub state: String,
    pub status: Option<i64>,
    pub problem: Option<String>,
}

pub struct Store {
    conn: Mutex<Connection>,
}

const MIGRATIONS: &[&str] = &[
    "CREATE TABLE cache (
         key        TEXT PRIMARY KEY,
         body       TEXT NOT NULL,
         etag       TEXT,
         fetched_at INTEGER NOT NULL
     );
     CREATE TABLE outbox (
         id         INTEGER PRIMARY KEY AUTOINCREMENT,
         method     TEXT NOT NULL,
         path       TEXT NOT NULL,
         body       TEXT,
         idem_key   TEXT NOT NULL,
         created_at INTEGER NOT NULL,
         attempts   INTEGER NOT NULL DEFAULT 0,
         next_at    INTEGER NOT NULL DEFAULT 0,
         state      TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'sending', 'failed')),
         status     INTEGER,
         problem    TEXT
     );
     CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    // The deadlines a notification was shown for, so each is shown once.
    "CREATE TABLE notified (
         assignment_id TEXT NOT NULL,
         due_at        TEXT NOT NULL,
         window        TEXT NOT NULL,
         PRIMARY KEY (assignment_id, due_at, window)
     );",
];

const COLUMNS: &str =
    "id, method, path, body, idem_key, created_at, attempts, next_at, state, status, problem";

fn row_of(row: &rusqlite::Row<'_>) -> rusqlite::Result<OutboxRow> {
    Ok(OutboxRow {
        id: row.get(0)?,
        method: row.get(1)?,
        path: row.get(2)?,
        body: row.get(3)?,
        idem_key: row.get(4)?,
        created_at: row.get(5)?,
        attempts: row.get(6)?,
        next_at: row.get(7)?,
        state: row.get(8)?,
        status: row.get(9)?,
        problem: row.get(10)?,
    })
}

/// A student's draft and the submit that follows it belong together: `/assignments/{id}/submission/...`.
fn assignment_prefix(path: &str) -> &str {
    path.find("/submission")
        .map_or(path, |at| &path[..at + "/submission".len()])
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::from(Connection::open(path)?)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::from(Connection::open_in_memory()?)
    }

    fn from(mut conn: Connection) -> rusqlite::Result<Self> {
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (index, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.execute_batch(&format!("PRAGMA user_version = {}", index + 1))?;
            tx.commit()?;
        }
        // A row that was being sent when the app stopped is sent again; its key makes that safe.
        conn.execute(
            "UPDATE outbox SET state = 'pending' WHERE state = 'sending'",
            [],
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A panic while the lock was held cannot leave SQLite half-written; carry on with the data.
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // --- cache -----------------------------------------------------------------------------------

    pub fn cache_get(&self, key: &str) -> rusqlite::Result<Option<CacheEntry>> {
        self.conn()
            .query_row(
                "SELECT body, etag, fetched_at FROM cache WHERE key = ?1",
                [key],
                |r| {
                    Ok(CacheEntry {
                        body: r.get(0)?,
                        etag: r.get(1)?,
                        fetched_at: r.get(2)?,
                    })
                },
            )
            .optional()
    }

    pub fn cache_put(
        &self,
        key: &str,
        body: &str,
        etag: Option<&str>,
        now: i64,
    ) -> rusqlite::Result<()> {
        self.conn().execute(
            "INSERT INTO cache (key, body, etag, fetched_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (key) DO UPDATE SET body = ?2, etag = ?3, fetched_at = ?4",
            params![key, body, etag, now],
        )?;
        Ok(())
    }

    /// The server said "unchanged": the copy is as fresh as the answer.
    pub fn cache_touch(&self, key: &str, now: i64) -> rusqlite::Result<()> {
        self.conn().execute(
            "UPDATE cache SET fetched_at = ?2 WHERE key = ?1",
            params![key, now],
        )?;
        Ok(())
    }

    pub fn cache_delete(&self, key: &str) -> rusqlite::Result<()> {
        self.conn()
            .execute("DELETE FROM cache WHERE key = ?1", [key])?;
        Ok(())
    }

    // --- outbox ----------------------------------------------------------------------------------

    /// Queues a write. A `PUT` replaces the one still waiting for the same path (last write wins for a
    /// draft), unless a `POST` for the same assignment was queued after it: that one was meant to send
    /// the older text.
    pub fn enqueue(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
        idem_key: &str,
        now_ms: i64,
    ) -> rusqlite::Result<i64> {
        let conn = self.conn();
        if method == "PUT" {
            let waiting: Option<i64> = conn
                .query_row(
                    "SELECT max(id) FROM outbox WHERE state = 'pending' AND method = 'PUT' AND path = ?1",
                    [path],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            if let Some(id) = waiting {
                let like = format!(
                    "{}%",
                    assignment_prefix(path)
                        .replace('%', "\\%")
                        .replace('_', "\\_")
                );
                let later: i64 = conn.query_row(
                    "SELECT count(*) FROM outbox WHERE id > ?1 AND state IN ('pending', 'sending')
                       AND method <> 'PUT' AND path LIKE ?2 ESCAPE '\\'",
                    params![id, like],
                    |r| r.get(0),
                )?;
                if later == 0 {
                    conn.execute(
                        "UPDATE outbox SET body = ?2, idem_key = ?3, created_at = ?4, attempts = 0, next_at = 0
                         WHERE id = ?1",
                        params![id, body, idem_key, now_ms],
                    )?;
                    return Ok(id);
                }
            }
        }
        conn.execute(
            "INSERT INTO outbox (method, path, body, idem_key, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![method, path, body, idem_key, now_ms],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// The oldest row that is due. Rows that failed for good, or are being sent, are not due.
    pub fn next_due(&self, now_s: i64) -> rusqlite::Result<Option<OutboxRow>> {
        self.conn()
            .query_row(
                &format!("SELECT {COLUMNS} FROM outbox WHERE state = 'pending' AND next_at <= ?1 ORDER BY id LIMIT 1"),
                [now_s],
                row_of,
            )
            .optional()
    }

    /// When the first waiting row is due, `None` when nothing waits.
    pub fn next_wake(&self) -> rusqlite::Result<Option<i64>> {
        self.conn().query_row(
            "SELECT min(next_at) FROM outbox WHERE state = 'pending'",
            [],
            |r| r.get(0),
        )
    }

    pub fn has_waiting(&self) -> rusqlite::Result<bool> {
        let n: i64 = self.conn().query_row(
            "SELECT count(*) FROM outbox WHERE state IN ('pending', 'sending')",
            [],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    pub fn mark_sending(&self, id: i64) -> rusqlite::Result<()> {
        self.conn()
            .execute("UPDATE outbox SET state = 'sending' WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn mark_retry(&self, id: i64, attempts: i64, next_at: i64) -> rusqlite::Result<()> {
        self.conn().execute(
            "UPDATE outbox SET state = 'pending', attempts = ?2, next_at = ?3 WHERE id = ?1",
            params![id, attempts, next_at],
        )?;
        Ok(())
    }

    pub fn mark_failed(&self, id: i64, status: i64, problem: Option<&str>) -> rusqlite::Result<()> {
        self.conn().execute(
            "UPDATE outbox SET state = 'failed', status = ?2, problem = ?3 WHERE id = ?1",
            params![id, status, problem],
        )?;
        Ok(())
    }

    pub fn outbox_delete(&self, id: i64) -> rusqlite::Result<()> {
        self.conn()
            .execute("DELETE FROM outbox WHERE id = ?1", [id])?;
        Ok(())
    }

    /// "Send now": nothing waits for a back-off any more.
    pub fn retry_all_now(&self) -> rusqlite::Result<()> {
        self.conn()
            .execute("UPDATE outbox SET next_at = 0 WHERE state = 'pending'", [])?;
        Ok(())
    }

    pub fn outbox_list(&self) -> rusqlite::Result<Vec<OutboxRow>> {
        let conn = self.conn();
        let mut statement = conn.prepare(&format!("SELECT {COLUMNS} FROM outbox ORDER BY id"))?;
        statement.query_map([], row_of)?.collect()
    }

    // --- meta and wipe ---------------------------------------------------------------------------

    pub fn meta_get(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.conn()
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()
    }

    pub fn meta_set(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn().execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = ?2",
            params![key, value],
        )?;
        Ok(())
    }

    /// True the first time a notice is recorded, false when it was shown before.
    pub fn notified_insert(
        &self,
        assignment_id: &str,
        due_at: &str,
        window: &str,
    ) -> rusqlite::Result<bool> {
        let changed = self.conn().execute(
            "INSERT OR IGNORE INTO notified (assignment_id, due_at, window) VALUES (?1, ?2, ?3)",
            params![assignment_id, due_at, window],
        )?;
        Ok(changed == 1)
    }

    /// Everything of the person who is leaving: what they read, what they had not sent, what they were told.
    /// The language is the device's, not theirs, and stays.
    pub fn wipe(&self) -> rusqlite::Result<()> {
        self.conn().execute_batch(
            "DELETE FROM cache; DELETE FROM outbox; DELETE FROM notified; DELETE FROM meta WHERE key <> 'language';",
        )
    }
}

#[cfg(test)]
mod tests;
