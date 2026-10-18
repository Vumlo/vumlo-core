//! What every command that needs the configuration or the database starts from.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use module_api::{ModuleContext, ModuleDef};
use sqlx::PgPool;
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;
use vumlo_config::{Loaded, Options, Settings};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The optional modules this binary was built with: one line per Cargo feature, the same names
/// as on `server` (ADR-0054).
pub fn compiled() -> Vec<ModuleDef> {
    vec![
        #[cfg(feature = "dummy")]
        vumlo_module_dummy::def(),
    ]
}

pub fn compiled_names() -> Vec<&'static str> {
    compiled().iter().map(|m| m.name).collect()
}

/// The migrators of the compiled-in modules, each with its own migrations table.
pub fn migrators() -> Vec<(&'static str, &'static Migrator)> {
    compiled()
        .iter()
        .filter_map(|m| m.migrator.map(|migrator| (m.name, migrator)))
        .collect()
}

/// The configuration, read strictly: whatever the file, the environment and the flags say, it
/// must be valid for this binary.
pub fn load(options: &Options) -> anyhow::Result<Loaded> {
    vumlo_config::load_process(options, &compiled_names()).context("invalid configuration")
}

pub async fn connect(settings: &Settings) -> anyhow::Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(5))
        .connect(settings.database.url.expose())
        .await
        .context("cannot connect to the database")
}

pub fn module_context(settings: &Settings, pool: &PgPool, name: &str) -> ModuleContext {
    ModuleContext {
        db: pool.clone(),
        settings: settings.module_state.get(name).cloned().unwrap_or_default(),
    }
}

/// `path` relative to the directory of the config file.
pub fn from_config_dir(settings: &Settings, path: &std::path::Path) -> PathBuf {
    settings.base_dir.join(path)
}
