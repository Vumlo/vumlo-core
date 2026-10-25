//! Plugins: external HTTP services the core does not trust (ADR-0066).
//!
//! An administrator installs a plugin with `vumlo-cli plugin install`: the core reads its manifest,
//! the administrator grants scopes, and the core gives it a token and derives a secret. From then
//! on the core sends it the events it asked for, signed (`delivery`), and it calls the plugin API
//! with its token (`routes`), where every handler checks a scope. This crate writes only the
//! schema `plugins`.

mod delivery;
mod error;
mod manifest;
mod model;
mod repo;
mod routes;
mod secret;
mod service;
mod state;

use events::{HandlerError, Handlers};
use sqlx::PgPool;

pub use delivery::Deliverer;
pub use error::PluginsError;
pub use manifest::parse_manifest;
pub use model::{
    API_VERSION, Install, InstallStatus, InstallSummary, Installed, PluginManifest, PluginToken,
    SLOTS, SUBSCRIBABLE, UiSlot,
};
pub use routes::router;
pub use secret::{SecretError, SecretKey, derive_secret, sign};
pub use service::{
    check_base_url, fan_out, fetch_manifest, get, grant, install, list, put_report, remove,
    reports, set_status,
};
pub use state::PluginsState;

/// The outbox handlers that owe a delivery to every enabled plugin subscribed to an event. A
/// delivery is unique per plugin and event, so an event handled twice owes once.
pub fn event_handlers(pool: PgPool) -> Handlers {
    SUBSCRIBABLE
        .iter()
        .fold(Handlers::default(), |handlers, kind| {
            let pool = pool.clone();
            handlers.on(kind, move |event| {
                let pool = pool.clone();
                Box::pin(async move {
                    let mut tx = pool
                        .begin()
                        .await
                        .map_err(|e| HandlerError(e.to_string()))?;
                    fan_out(&mut tx, &event)
                        .await
                        .map_err(|e| HandlerError(e.to_string()))?;
                    tx.commit().await.map_err(|e| HandlerError(e.to_string()))
                })
            })
        })
}

#[cfg(test)]
mod tests;
