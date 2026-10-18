//! The audit log and the outbox; delivery (the worker), webhooks and jobs of ADR-0007 are not built yet.

mod model;
mod repo;
mod routes;
mod service;

pub use model::{AuditEntry, Event, HandlerError, HandlerFuture, Handlers, OutboxEvent};
pub use routes::router;
pub use service::{
    EventsError, audit, audit_admin, audit_cli, emit, list_audit, process_once, run_worker,
};
