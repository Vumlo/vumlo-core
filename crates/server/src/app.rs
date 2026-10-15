use std::time::Duration;

use sqlx::PgPool;
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;

use crate::settings::Settings;

pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// Shorter than the request timeout, so an unreachable database surfaces as 503, not 408.
const DB_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn connect(settings: &Settings) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(settings.database.max_connections)
        .acquire_timeout(DB_ACQUIRE_TIMEOUT)
        .connect(&settings.database.url)
        .await
}
