//! Plugins: external HTTP services the core does not trust (ADR-0066).
//!
//! An administrator installs a plugin with `vumlo-cli plugin install`: the core reads its manifest,
//! the administrator grants scopes, and the core gives it a token and derives a secret. From then
//! on the core sends it the events it asked for, signed (`delivery`), and it calls the plugin API
//! with its token (`routes`), where every handler checks a scope. What an administrator may
//! configure is declared in the manifest and kept per install (`settings`, ADR-0072). This crate
//! writes only the schema `plugins`.

mod delivery;
mod error;
mod manifest;
mod model;
mod proxy;
mod repo;
mod routes;
mod secret;
mod service;
mod settings;
mod state;
mod upgrade;

use events::{HandlerError, Handlers};
use sqlx::PgPool;

pub use delivery::Deliverer;
pub use error::PluginsError;
pub use manifest::parse_manifest;
pub use model::{
    API_VERSION, Install, InstallStatus, InstallSummary, Installed, PluginManifest, PluginToken,
    ReportSubject, SLOTS, SUBSCRIBABLE, UiSlot,
};
pub use proxy::{
    MAX_REQUEST_BYTES as PROXY_MAX_REQUEST_BYTES, openapi as proxy_openapi, router as proxy_router,
};
pub use repo::{CallCount, CallRow};
pub use routes::router;
pub use secret::{ProxyClaims, SecretError, SecretKey, derive_secret, proxy_signature, sign};
pub use service::{
    CallReport, SettingView, assignment_reports, calls, check_base_url, course_reports,
    delete_report, fan_out, fetch_manifest, get, get_setting, grant, install, list, list_settings,
    put_report, remove, reports, rotate_token, set_setting, set_status, settings_for_plugin,
    unset_setting,
};
pub use settings::{ItemSpec, ScalarKind, SettingKind, SettingScope, SettingSpec};
pub use state::PluginsState;
pub use upgrade::{DroppedValue, UpgradePlan, Upgraded, apply as upgrade, plan as plan_upgrade};

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
