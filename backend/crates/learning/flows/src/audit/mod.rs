//! The audit log as the administrator reads it: every row with the names it points at (ADR-0053).

mod model;
mod repo;
mod routes;
mod service;

pub use model::{AuditFilter, AuditRow};
pub use routes::router;
pub use service::list_audit;
