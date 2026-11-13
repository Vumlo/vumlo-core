//! Stored bytes and their description (ADR-0069). No routes, no actor: the layers above decide who
//! may upload or read a file and which owner references it.

mod mime;
mod model;
mod repo;
mod service;
mod storage;
mod verdict;

pub use mime::{clean_name, is_image, is_inline, sniff};
pub use model::{File, Stored};
pub use object_store::GetResult;
pub use service::{
    FilesError, candidates, delete, drop_object, forget, get, insert, lock, put, read, try_lock,
};
pub use storage::Storage;
pub use verdict::FileVerdict;
