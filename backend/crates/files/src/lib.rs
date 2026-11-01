//! Stored bytes and their description (ADR-0069). No routes, no actor: the layers above decide who
//! may upload or read a file and which owner references it.

mod mime;
mod model;
mod repo;
mod service;
mod storage;

pub use mime::{clean_name, is_inline, sniff};
pub use model::{File, Stored};
pub use object_store::GetResult;
pub use service::{FilesError, candidates, delete, get, insert, put, read};
pub use storage::Storage;
