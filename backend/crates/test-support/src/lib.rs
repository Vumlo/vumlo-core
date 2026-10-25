//! A throwaway database for one test: created before the body, dropped after it.
//!
//! This replaces `#[sqlx::test]`. Its test pools take their connections from one master pool of
//! 20 permits that every running test shares, and a pool keeps what it took until it is dropped.
//! With more than about a dozen tests running at once the permits run out: a request waits 30
//! seconds for a connection and ends as `408`, while every database session sits idle (audit
//! 014, A-10). Here each test has its own pool, bounded only by PostgreSQL's `max_connections`
//! (`compose.yaml` raises it to 300).
//!
//! It is a crate of its own, a dev-dependency of everything that tests against a database, and the
//! release archive leaves it out (`RELEASING.md`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use db::MIGRATOR;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, Connection, Executor, PgConnection, PgPool};
use uuid::Uuid;

/// Enough for the server (two at a time) and a test that holds a transaction open while a second
/// writer waits behind it.
const POOL_SIZE: u32 = 5;
/// Databases of tests that failed (their body panicked) are kept for inspection, then swept.
const KEEP_FAILED_FOR: Duration = Duration::from_secs(3600);
const PREFIX: &str = "t_";

static SWEPT: AtomicBool = AtomicBool::new(false);

/// `DATABASE_URL` from the environment, or from the workspace `.env`.
fn database_url() -> String {
    if let Ok(url) = std::env::var("DATABASE_URL") {
        return url;
    }
    let env_file = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../.env"))
        .expect("DATABASE_URL is not set and there is no .env");
    env_file
        .lines()
        .find_map(|line| line.strip_prefix("DATABASE_URL="))
        .expect("DATABASE_URL in .env")
        .trim()
        .to_owned()
}

/// The server and credentials of `DATABASE_URL`, connected to the maintenance database, which
/// always exists (the database named in the URL may not).
fn admin_options() -> PgConnectOptions {
    database_url()
        .parse::<PgConnectOptions>()
        .expect("DATABASE_URL is a PostgreSQL URL")
        .database("postgres")
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Runs `body` against a new database with every migration applied.
pub async fn db_test(body: impl AsyncFnOnce(PgPool)) {
    run(true, body).await;
}

/// Runs `body` against a new database with no migration applied.
pub async fn bare_db_test(body: impl AsyncFnOnce(PgPool)) {
    run(false, body).await;
}

async fn run(migrated: bool, body: impl AsyncFnOnce(PgPool)) {
    let admin = admin_options();
    if !SWEPT.swap(true, Ordering::SeqCst) {
        sweep(&admin).await;
    }
    let name = format!("{PREFIX}{}_{}", unix_seconds(), Uuid::now_v7().simple());
    // DDL cannot take a bound identifier. The name is made here from a number and a UUID.
    execute(&admin, AssertSqlSafe(format!("CREATE DATABASE \"{name}\""))).await;
    let pool = PgPoolOptions::new()
        .max_connections(POOL_SIZE)
        .idle_timeout(Some(Duration::from_secs(2)))
        .connect_with(admin.clone().database(&name))
        .await
        .expect("connect to the test database");
    if migrated {
        MIGRATOR.run(&pool).await.expect("apply the migrations");
    }
    body(pool.clone()).await;
    // A pool still held by a spawned server only has to stop handing out connections; the drop
    // below ends any session that is left.
    let _ = tokio::time::timeout(Duration::from_secs(10), pool.close()).await;
    execute(
        &admin,
        AssertSqlSafe(format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)")),
    )
    .await;
}

async fn execute(options: &PgConnectOptions, sql: AssertSqlSafe<String>) {
    let mut conn = PgConnection::connect_with(options)
        .await
        .expect("connect to the maintenance database");
    conn.execute(sql).await.expect("run the statement");
    let _ = conn.close().await;
}

/// Drops the databases of earlier failed tests once they are old enough.
async fn sweep(admin: &PgConnectOptions) {
    let Ok(mut conn) = PgConnection::connect_with(admin).await else {
        return;
    };
    let names: Vec<String> =
        sqlx::query_scalar("SELECT datname FROM pg_database WHERE datname LIKE 't\\_%'")
            .fetch_all(&mut conn)
            .await
            .unwrap_or_default();
    let now = unix_seconds();
    for name in names {
        let made = name
            .strip_prefix(PREFIX)
            .and_then(|rest| rest.split('_').next())
            .and_then(|seconds| seconds.parse::<u64>().ok());
        if made.is_some_and(|made| now.saturating_sub(made) > KEEP_FAILED_FOR.as_secs()) {
            let _ = conn
                .execute(AssertSqlSafe(format!(
                    "DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"
                )))
                .await;
        }
    }
    let _ = conn.close().await;
}

/// Returns once a session of this database waits for a lock, so a test can commit the first
/// writer knowing the second one is really queued behind it. Fails if the writer finished instead
/// (it waited for nothing) or if nobody waits within ten seconds.
pub async fn wait_until_blocked<T>(pool: &PgPool, writer: &tokio::task::JoinHandle<T>) {
    // A system view, not our schema, so there is nothing for sqlx to check at compile time.
    let waiting = "SELECT count(*) FROM pg_stat_activity
                   WHERE wait_event_type = 'Lock' AND datname = current_database()";
    let mut conn = pool.acquire().await.expect("a connection to poll with");
    let waited = tokio::time::timeout(Duration::from_secs(10), async {
        while !writer.is_finished()
            && sqlx::query_scalar::<_, i64>(waiting)
                .fetch_one(&mut *conn)
                .await
                .expect("poll pg_stat_activity")
                < 1
        {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(waited.is_ok(), "no writer waited for a lock");
    assert!(
        !writer.is_finished(),
        "the second writer finished without waiting for a lock"
    );
}
