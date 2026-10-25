//! What every command that needs the configuration or the database starts from.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use module_api::{Module, ModuleState, NoActors};
use sqlx::PgPool;
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;
use vumlo_config::{Loaded, ModuleSpec, Options, Settings};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The optional modules this binary was built with: one line per Cargo feature, the same names
/// as on `server` (ADR-0054, ADR-0066).
pub fn compiled() -> Vec<&'static dyn Module> {
    vec![
        #[cfg(feature = "dummy")]
        vumlo_module_dummy::module(),
    ]
}

pub fn compiled_names() -> Vec<&'static str> {
    compiled().iter().map(|m| m.manifest().id).collect()
}

/// What the configuration is checked against.
pub fn specs() -> Vec<ModuleSpec> {
    compiled().iter().map(|m| m.manifest().spec()).collect()
}

/// The migrators of the compiled-in modules, each with its history inside its own schema.
pub fn migrators() -> Vec<(&'static str, &'static Migrator)> {
    compiled()
        .iter()
        .filter_map(|m| m.migrations().map(|migrator| (m.manifest().id, migrator)))
        .collect()
}

/// The configuration, read strictly: whatever the file, the environment and the flags say, it
/// must be valid for this binary.
pub fn load(options: &Options) -> anyhow::Result<Loaded> {
    vumlo_config::load_process(options, &specs()).context("invalid configuration")
}

pub async fn connect(settings: &Settings) -> anyhow::Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(5))
        .connect(settings.database.url.expose())
        .await
        .context("cannot connect to the database")
}

/// The state a module's subcommand and doctor check get. Nobody is signed in on the command line.
pub fn module_state(
    module: &'static dyn Module,
    settings: &Settings,
    pool: &PgPool,
) -> ModuleState {
    ModuleState::new(
        module.manifest(),
        pool.clone(),
        settings
            .module_state
            .get(module.manifest().id)
            .cloned()
            .unwrap_or_default(),
        Arc::new(NoActors),
    )
}

/// `path` relative to the directory of the config file.
pub fn from_config_dir(settings: &Settings, path: &std::path::Path) -> PathBuf {
    settings.base_dir.join(path)
}
