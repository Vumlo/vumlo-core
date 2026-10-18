//! The contract between the core and an optional module (ADR-0054).
//!
//! A module is its own crate, compiled in by a Cargo feature of the same name on `server` and
//! `vumlo-cli`. It describes itself with one [`ModuleDef`]; the core mounts the routes, starts the
//! workers, runs the migrations, adds the subcommand and the doctor checks, and only for a module
//! that is compiled in. A module never depends on `server`, and nothing here names a module.

mod outbound;

use std::future::Future;
use std::pin::Pin;

use sqlx::PgPool;
use sqlx::migrate::Migrator;
use tokio::task::JoinHandle;
use vumlo_config::ResolvedModule;

pub use outbound::{OutboundClient, OutboundError};

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// What a module gets from the core: the pool and its own settings.
#[derive(Clone)]
pub struct ModuleContext {
    pub db: PgPool,
    pub settings: ResolvedModule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
    Skip,
}

/// One line of `vumlo-cli doctor`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Check {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

impl Check {
    pub fn new(name: impl Into<String>, status: CheckStatus, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status,
            detail: detail.into(),
        }
    }
}

/// A subcommand of `vumlo-cli module <name>`.
#[derive(Clone, Copy)]
pub struct CliExt {
    pub command: fn() -> clap::Command,
    pub run: fn(clap::ArgMatches, ModuleContext) -> BoxFuture<anyhow::Result<()>>,
}

#[derive(Clone, Copy)]
pub struct ModuleDef {
    /// The name in `[modules]` and the registry of `vumlo-config`.
    pub name: &'static str,
    /// Its own migrations, in their own table (`_sqlx_migrations_<name>`), applied whenever the
    /// module is compiled in, whether it is enabled or not.
    pub migrator: Option<&'static Migrator>,
    /// The routes, relative to `/api/v1/<name>`. Mounted only while the module is enabled.
    pub router: fn(ModuleContext) -> axum::Router,
    /// The OpenAPI document of those routes (paths relative to `/api/v1/<name>`).
    pub openapi: fn() -> utoipa::openapi::OpenApi,
    /// Background work, started only while the module is enabled.
    pub workers: fn(ModuleContext) -> Vec<JoinHandle<()>>,
    pub cli: Option<CliExt>,
    /// Checks for `doctor`, also while the module is compiled in but disabled: it can tell that
    /// data of a disabled module is still stored.
    pub doctor: fn(ModuleContext) -> BoxFuture<Vec<Check>>,
}
