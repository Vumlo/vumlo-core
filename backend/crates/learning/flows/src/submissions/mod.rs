//! What needs several modules to save or submit student work (ADR-0093, ADR-0069): the files a
//! draft or an attempt names are judged by `files::check`, which reads `content` and `assessment`,
//! so neither of them can do it. `assessment` validates and locks before (`begin_*`), the check
//! locks and judges the files, and `assessment` writes after (`finish_*`), all on the connection of
//! one transaction.

mod routes;
mod service;

pub use routes::router;
pub use service::{auto_submit, save_draft, submit};

#[cfg(test)]
mod tests;
