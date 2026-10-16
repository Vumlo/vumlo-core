//! The audit log and the outbox; delivery (the worker), webhooks and jobs of ADR-0007 are not built yet.

mod model;
mod repo;
mod service;

pub use model::{AuditEntry, Event};
pub use service::{EventsError, audit, audit_admin, emit};
