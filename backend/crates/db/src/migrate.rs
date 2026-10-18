//! The embedded migrations and what a database has applied of them.
//!
//! The core migrations share one table, `_sqlx_migrations`. Every optional module keeps its own
//! table (`_sqlx_migrations_<module>`, set on its `Migrator`), so a module that is compiled out
//! can never make the core refuse to start, and a database that is newer than the binary is
//! always the core's doing.

use std::collections::HashSet;
use std::fmt;

use sqlx::migrate::{Migrate, MigrateError, Migrator};
use sqlx::{PgConnection, PgPool};

/// The migrations of the core: `backend/migrations/`.
pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

#[derive(Debug)]
pub enum MigrationCheckError {
    Db(sqlx::Error),
    Migrate(MigrateError),
    Pending(usize),
    /// The database has migrations this binary does not know: it was migrated by a newer release.
    Newer {
        label: String,
        versions: Vec<i64>,
    },
}

impl fmt::Display for MigrationCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Db(e) => write!(f, "cannot read the migration state: {e}"),
            Self::Migrate(e) => write!(f, "cannot read the migration state: {e}"),
            Self::Pending(n) => write!(
                f,
                "{n} database migration(s) pending; run `vumlo-cli db migrate` (or `vumlo-server migrate`) first"
            ),
            Self::Newer { label, versions } => write!(
                f,
                "the {label} schema is newer than this binary: it has migration(s) {versions:?} \
                 that this release does not contain; update the binary instead of migrating"
            ),
        }
    }
}

impl std::error::Error for MigrationCheckError {}

impl From<sqlx::Error> for MigrationCheckError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

impl From<MigrateError> for MigrationCheckError {
    fn from(e: MigrateError) -> Self {
        Self::Migrate(e)
    }
}

/// What one migrations table holds compared to the migrations of the binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub label: String,
    pub table: String,
    /// The table exists in the database.
    pub exists: bool,
    pub applied: Vec<i64>,
    /// In the binary, not applied yet.
    pub pending: Vec<i64>,
    /// Applied, but not in the binary.
    pub newer: Vec<i64>,
    /// A migration that started and did not finish.
    pub dirty: Option<i64>,
    /// The newest migration the binary contains.
    pub latest: Option<i64>,
}

/// Reads the state of `migrator`'s table. Never changes the database.
pub async fn status(
    conn: &mut PgConnection,
    label: &str,
    migrator: &Migrator,
) -> Result<Status, sqlx::Error> {
    let table = migrator.table_name.to_string();
    let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(&table)
        .fetch_one(&mut *conn)
        .await?;
    let (applied, dirty) = if exists {
        let applied = conn
            .list_applied_migrations(&table)
            .await
            .map_err(into_sqlx)?;
        let dirty = conn.dirty_version(&table).await.map_err(into_sqlx)?;
        (applied.into_iter().map(|m| m.version).collect(), dirty)
    } else {
        (Vec::<i64>::new(), None)
    };
    let known: Vec<i64> = migrator
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
        .map(|m| m.version)
        .collect();
    let applied_set: HashSet<i64> = applied.iter().copied().collect();
    let known_set: HashSet<i64> = known.iter().copied().collect();
    Ok(Status {
        label: label.to_owned(),
        table,
        exists,
        pending: known
            .iter()
            .copied()
            .filter(|v| !applied_set.contains(v))
            .collect(),
        newer: applied
            .iter()
            .copied()
            .filter(|v| !known_set.contains(v))
            .collect(),
        latest: known.iter().copied().max(),
        applied,
        dirty,
    })
}

fn into_sqlx(e: MigrateError) -> sqlx::Error {
    sqlx::Error::Protocol(e.to_string())
}

/// The optional modules whose migrations table exists in the database, by module name: the
/// modules that were ever migrated here, compiled in now or not. Their data is never deleted.
pub async fn migrated_modules(conn: &mut PgConnection) -> Result<Vec<String>, sqlx::Error> {
    let tables: Vec<String> = sqlx::query_scalar(
        r"SELECT tablename::text FROM pg_catalog.pg_tables
          WHERE schemaname = current_schema() AND tablename LIKE '\_sqlx\_migrations\_%'
          ORDER BY tablename",
    )
    .fetch_all(conn)
    .await?;
    Ok(tables
        .iter()
        .filter_map(|table| table.strip_prefix("_sqlx_migrations_"))
        .map(str::to_owned)
        .collect())
}

/// Fails if a migration of the core has not been applied, or the core schema is newer than the
/// binary. Never changes the database.
pub async fn ensure_migrated(pool: &PgPool) -> Result<(), MigrationCheckError> {
    ensure_migrated_with(pool, &[]).await
}

/// Like [`ensure_migrated`], and also for the migrators of the optional modules compiled in.
pub async fn ensure_migrated_with(
    pool: &PgPool,
    modules: &[(&str, &Migrator)],
) -> Result<(), MigrationCheckError> {
    let mut conn = pool.acquire().await?;
    let mut pending = 0;
    for (label, migrator) in std::iter::once(("core", &MIGRATOR)).chain(modules.iter().copied()) {
        let state = status(&mut conn, label, migrator).await?;
        if !state.newer.is_empty() {
            return Err(MigrationCheckError::Newer {
                label: state.label,
                versions: state.newer,
            });
        }
        if let Some(version) = state.dirty {
            return Err(MigrateError::Dirty(version).into());
        }
        pending += state.pending.len();
    }
    if pending == 0 {
        Ok(())
    } else {
        Err(MigrationCheckError::Pending(pending))
    }
}

/// Applies what `migrator` has pending and returns the versions applied now (empty when the
/// database was up to date). Refuses a schema that is newer than the binary.
pub async fn migrate(
    pool: &PgPool,
    label: &str,
    migrator: &Migrator,
) -> Result<Vec<i64>, MigrationCheckError> {
    let mut conn = pool.acquire().await?;
    let before = status(&mut conn, label, migrator).await?;
    drop(conn);
    if !before.newer.is_empty() {
        return Err(MigrationCheckError::Newer {
            label: before.label,
            versions: before.newer,
        });
    }
    if before.pending.is_empty() && before.dirty.is_none() {
        return Ok(Vec::new());
    }
    migrator.run(pool).await?;
    Ok(before.pending)
}
