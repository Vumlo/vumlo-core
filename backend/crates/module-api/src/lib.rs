//! The contract between the core and an optional module (ADR-0054, ADR-0066).
//!
//! A module is its own crate, compiled in by a Cargo feature of the same name on `server` and
//! `vumlo-cli`. It implements [`Module`]: a static [`Manifest`], its migrations, and `register`,
//! which adds routes, event handlers and background jobs to a [`Registrar`]. The core mounts the
//! routes, runs the migrations, starts the jobs, adds the subcommand and the doctor checks, and
//! only for a module that is compiled in. A module never depends on `server`, and nothing here
//! names a module.

mod manifest;
mod order;
mod outbound;
mod registrar;
mod state;

use sqlx::migrate::Migrator;

pub use manifest::{Manifest, Permission, schema_of};
pub use order::{OrderError, init_order};
pub use outbound::{OutboundClient, OutboundError};
pub use registrar::{Job, Registered, Registrar};
pub use state::{DynActorSource, ModuleState, NoActors};

pub type BoxFuture<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;

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
    pub run: fn(clap::ArgMatches, ModuleState) -> BoxFuture<anyhow::Result<()>>,
}

/// An optional module. The core calls these in the order of [`init_order`].
pub trait Module: Send + Sync + 'static {
    fn manifest(&self) -> &'static Manifest;

    /// Its own migrations, made with [`scoped_migrator`]: they create the schema `mod_<id>` and
    /// keep their history inside it. Applied whenever the module is compiled in, enabled or not.
    fn migrations(&self) -> Option<&'static Migrator> {
        None
    }

    /// Adds the module's routes, event handlers and jobs. Called on every start of a process that
    /// needs them (the server for the routes, the worker for the jobs), so it builds values and
    /// does nothing else: no I/O, no spawning.
    fn register(&self, cx: &mut Registrar);

    fn cli(&self) -> Option<CliExt> {
        None
    }

    /// Checks for `doctor`, also while the module is compiled in but disabled: it can tell that
    /// data of a disabled module is still stored.
    fn doctor(&self, _state: ModuleState) -> BoxFuture<Vec<Check>> {
        Box::pin(async { Vec::new() })
    }
}

/// Puts `migrator` in the schema of module `id`: the schema `mod_<id>` is created first and the
/// history table is `mod_<id>._sqlx_migrations`, so a module never writes the core's tables or the
/// core's history (ADR-0066).
pub fn scoped_migrator(id: &str, mut migrator: Migrator) -> Migrator {
    let schema = schema_of(id);
    migrator.create_schema(schema.clone());
    migrator.dangerous_set_table_name(format!("{schema}._sqlx_migrations"));
    migrator
}

#[cfg(test)]
mod tests;
