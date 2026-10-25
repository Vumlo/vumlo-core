//! The administrator's statistics across the institution (ADR-0053).

mod model;
mod routes;
mod service;

pub use model::TermUsage;
pub use routes::router;
pub use service::term_stats;
