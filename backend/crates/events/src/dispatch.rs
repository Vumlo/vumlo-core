//! Synchronous subscribers: functions that run inside the transaction of the change that emitted
//! an event (ADR-0093). The composition root builds the registry from what each module offers and
//! installs it once; the registry never changes afterwards. The outbox (`Handlers`) stays the way
//! to reach consumers that may fail and be retried.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use sqlx::PgConnection;

use super::model::Event;
use super::repo;
use super::service::EventsError;

/// Why a subscriber failed. The operation that emitted the event fails with it and rolls back, so
/// the text holds no secret and no personal data.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct SubscriberError(pub String);

pub type SyncFuture<'a> = Pin<Box<dyn Future<Output = Result<(), SubscriberError>> + Send + 'a>>;
type SyncHandler =
    Arc<dyn for<'a> Fn(&'a mut PgConnection, &'a Event) -> SyncFuture<'a> + Send + Sync>;

/// Which functions run, in the caller's transaction, for which event type. A type may have several
/// subscribers, in the order they were added; the first error stops the rest and fails the
/// operation. A subscriber is not retried: it runs once per emit, so it is written to give the
/// same state when run again by a rebuild.
#[derive(Clone, Default)]
pub struct Subscribers(HashMap<&'static str, Vec<SyncHandler>>);

impl Subscribers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on<F>(mut self, kind: &'static str, handler: F) -> Self
    where
        F: for<'a> Fn(&'a mut PgConnection, &'a Event) -> SyncFuture<'a> + Send + Sync + 'static,
    {
        self.0.entry(kind).or_default().push(Arc::new(handler));
        self
    }

    pub fn merge(mut self, other: Subscribers) -> Self {
        for (kind, mut handlers) in other.0 {
            self.0.entry(kind).or_default().append(&mut handlers);
        }
        self
    }

    fn get(&self, kind: &str) -> &[SyncHandler] {
        self.0.get(kind).map_or(&[], Vec::as_slice)
    }
}

static REGISTRY: OnceLock<Subscribers> = OnceLock::new();

/// Installs the registry of this process. Called once by the composition root; a second call is an
/// error, because a registry that could change would make `emit` depend on who called last.
pub fn install(subscribers: Subscribers) -> Result<(), EventsError> {
    REGISTRY
        .set(subscribers)
        .map_err(|_| EventsError::AlreadyInstalled)
}

/// The installed registry, or the error that tells a process which forgot to install one.
pub(super) fn installed() -> Result<&'static Subscribers, EventsError> {
    REGISTRY.get().ok_or(EventsError::NoSubscribers)
}

/// Installs `build()` unless a registry is already there. For test binaries only, where many tests
/// of one process start the application.
#[cfg(feature = "testing")]
pub fn ensure_installed(build: impl FnOnce() -> Subscribers) {
    REGISTRY.get_or_init(build);
}

/// Writes `event` to the outbox, then runs the subscribers of its type on the same connection.
/// This is [`crate::emit`] with the registry given instead of installed.
pub async fn emit_with(
    conn: &mut PgConnection,
    event: Event,
    subscribers: &Subscribers,
) -> Result<(), EventsError> {
    repo::insert_outbox(conn, &event).await?;
    for subscriber in subscribers.get(event.kind()) {
        subscriber(conn, &event).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
