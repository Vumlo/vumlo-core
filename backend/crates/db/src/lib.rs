//! What the server and `vumlo-cli` share about the database (ADR-0054): the migrations and their
//! state, the installation record, the CLI's audit row, and the value types of an account (an
//! email, a name, a password and how it is hashed), so both validate and hash identically.

mod audit;
pub mod identity;
mod installation;
mod migrate;
mod text_enum;

pub use audit::insert_cli_audit;
pub use installation::{ModeError, record_mode};
pub use migrate::{
    MIGRATOR, MigrationCheckError, Status, ensure_migrated, ensure_migrated_with, migrate,
    migrated_modules, status,
};
