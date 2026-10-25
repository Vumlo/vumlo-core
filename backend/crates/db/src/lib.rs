//! What the server and `vumlo-cli` share about the database (ADR-0054): the migrations and their
//! state, the installation record and the CLI's audit row.

mod audit;
mod installation;
mod migrate;

pub use audit::insert_cli_audit;
pub use installation::{ModeError, record_mode};
pub use migrate::{
    MIGRATOR, MigrationCheckError, Status, ensure_migrated, ensure_migrated_with, migrate,
    migrated_modules, status,
};
