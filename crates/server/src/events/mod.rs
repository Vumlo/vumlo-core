//! The audit log; the outbox, webhooks and jobs of ADR-0007 are not built yet.

mod model;
mod repo;
mod service;

pub use model::AuditEntry;
pub use service::{EventsError, audit, audit_admin};
