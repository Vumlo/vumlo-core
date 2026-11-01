//! Who may upload, read and use a file (ADR-0069). The bytes and their description live in the
//! `files` crate below; the references that say which item or submission uses a file live in
//! `content` and `assessment`. This module is the flow that reads both: it owns no schema.

mod model;
mod repo;
mod routes;
mod service;
mod sweep;

pub use model::{Owner, UsageOwner};
pub use routes::router;
pub use service::{
    FileError, authorize_upload, ensure_usable, first_non_image, open_for_read, register_upload,
};
pub use sweep::{run_sweeper, sweep};
