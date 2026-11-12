//! The audit log and the outbox; delivery (the worker), webhooks and jobs of ADR-0007 are not built yet.

mod dispatch;
mod model;
mod repo;
mod service;

pub use dispatch::{SubscriberError, Subscribers, SyncFuture, emit_with, install};
pub use model::{AuditEntry, Event, HandlerError, HandlerFuture, Handlers, OutboxEvent};

/// For the test binaries of other crates (`[dev-dependencies]` only).
#[cfg(feature = "testing")]
pub mod testing {
    pub use super::dispatch::ensure_installed;
}
pub use service::{
    EventsError, MAX_ATTEMPTS, audit, audit_admin, audit_cli, audit_plugin, backoff, emit,
    process_once, run_worker,
};
