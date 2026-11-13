//! Outgoing mail (ADR-0084 to ADR-0087): every mail is a row, one per recipient, queued in the
//! transaction of whatever asked for it and sent by one worker. The layer cannot read `auth`: the
//! caller above passes each recipient's address, and the layer copies it into the row and erases it
//! `mail.retention_days` after the message ends.

mod admin;
mod admin_routes;
mod compose;
mod error;
mod mailer;
mod model;
mod optout;
mod repo;
mod routes;
mod service;
mod state;
mod worker;

pub use error::MailError;
pub use mailer::{Mailer, Outgoing, SendError};
pub use model::{
    AccountMail, Address, Body, DedupeKey, Filter, JournalEntry, Kind, Limits, LinkPath,
    MessageState, ModelError, PluginMail, PluginOutcome, Recipient, Retried, RetryScope, Skipped,
    Status, Subject,
};
pub use routes::router;
pub use service::{
    cancel_pending, enqueue_account, enqueue_plugin, list, origin_queued, retry, status,
};
pub use state::MailState;
pub use worker::{Context, process_once, run_worker, sweep};

#[cfg(test)]
mod tests;
