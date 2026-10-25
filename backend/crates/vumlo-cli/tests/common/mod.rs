// Each test binary compiles this module and uses only some helpers.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sqlx::PgPool;
use tokio::net::TcpListener;
use vumlo_config::{FileSource, Settings, Sources};

// The per-test database harness (ADR-0036).
// Not every test binary needs both variants.
#[allow(unused_imports)]
pub use test_support::{bare_db_test, db_test};

/// Applies the migrations of the optional modules this build contains. `db_test` only migrates
/// the core, and the CLI treats a database with a module migration pending as not ready.
pub async fn migrate_modules(pool: &PgPool) {
    #[cfg(feature = "dummy")]
    vumlo_module_dummy::module()
        .migrations()
        .expect("the dummy module has migrations")
        .run(pool)
        .await
        .expect("the module's migrations");
    #[cfg(not(feature = "dummy"))]
    let _ = pool;
}

pub const ORIGIN: &str = "https://lms.test";

/// The minimal configuration file every test starts from.
pub const CONFIG: &str = r#"
[server]
public_origin = "https://lms.test"
[institution]
name = "Test University"
mode = "university"
"#;

pub struct Out {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// `DATABASE_URL` from the environment, or from the workspace `.env` as the harness does.
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

/// The URL of the throwaway database the harness created for this test.
pub fn test_database_url(pool: &PgPool) -> String {
    let url = database_url();
    let server = url
        .split('?')
        .next()
        .and_then(|u| u.rsplit_once('/'))
        .expect("url")
        .0;
    let database = pool
        .connect_options()
        .get_database()
        .expect("database")
        .to_owned();
    format!("{server}/{database}")
}

/// A directory with a `vumlo.toml`, and the means to run `vumlo-cli` in it against a database.
pub struct Env {
    pub dir: PathBuf,
    pub url: String,
}

impl Env {
    pub fn new(pool: &PgPool) -> Self {
        Self::with_config(pool, CONFIG)
    }

    pub fn with_config(pool: &PgPool, config: &str) -> Self {
        Self::at(test_database_url(pool), config)
    }

    pub fn at(url: String, config: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("vumlo-cli-test-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).expect("test directory");
        std::fs::write(dir.join("vumlo.toml"), config).expect("vumlo.toml");
        Self { dir, url }
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    pub async fn run(&self, args: &[&str]) -> Out {
        self.run_with(args, None, &[("DATABASE_URL", self.url.as_str())])
            .await
    }

    /// Runs the binary with exactly the environment `env` (plus `PATH`), in the directory.
    pub async fn run_with(&self, args: &[&str], stdin: Option<&str>, env: &[(&str, &str)]) -> Out {
        let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        let stdin = stdin.map(str::to_owned);
        let dir = self.dir.clone();
        tokio::task::spawn_blocking(move || run_binary(&dir, &args, stdin.as_deref(), &env))
            .await
            .expect("join")
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn run_binary(dir: &Path, args: &[String], stdin: Option<&str>, env: &[(String, String)]) -> Out {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vumlo-cli"));
    command
        .args(args)
        .current_dir(dir)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("run vumlo-cli");
    if let Some(text) = stdin {
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(text.as_bytes())
            .expect("write stdin");
    }
    let output = child.wait_with_output().expect("wait for vumlo-cli");
    Out {
        ok: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// The specs of the modules this build contains.
fn vumlo_cli_specs() -> Vec<vumlo_config::ModuleSpec> {
    #[cfg(feature = "dummy")]
    return vec![vumlo_module_dummy::module().manifest().spec()];
    #[cfg(not(feature = "dummy"))]
    return Vec::new();
}

/// The settings `CONFIG` gives for the database `url`: what the real server would load.
pub fn settings_for(url: &str, config: &str) -> Settings {
    vumlo_config::load(
        Sources {
            file: Some(FileSource::Text(config.to_owned())),
            env: BTreeMap::from([("DATABASE_URL".to_owned(), url.to_owned())]),
            overrides: Vec::new(),
        },
        &vumlo_cli_specs(),
    )
    .expect("test settings")
}

/// Starts the real server on a random port against `pool`; returns its base URL.
pub async fn spawn_server(pool: &PgPool, settings: Settings) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("address");
    let state = server::app::AppState::new(pool.clone(), settings).expect("app state");
    tokio::spawn(server::app::serve(
        listener,
        server::app::router(state).expect("router"),
        std::future::pending(),
    ));
    format!("http://{addr}")
}

/// A stand-in for the core that answers `/api/v1/health` with `body`.
pub async fn fake_core(body: serde_json::Value) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("address");
    let app = axum::Router::new().route(
        "/api/v1/health",
        axum::routing::get(move || {
            let body = body.clone();
            async move { axum::Json(body) }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://{addr}")
}

/// `(action, source, has no actor, resource_type)` of every audit row, oldest first.
pub async fn audit(pool: &PgPool) -> Vec<(String, String, bool, String)> {
    sqlx::query_as(
        "SELECT action, source, actor_id IS NULL, resource_type FROM events.audit_log ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .expect("audit rows")
}

pub async fn audit_meta(pool: &PgPool, action: &str) -> serde_json::Value {
    sqlx::query_scalar("SELECT meta FROM events.audit_log WHERE action = $1")
        .bind(action)
        .fetch_one(pool)
        .await
        .expect("audit meta")
}
