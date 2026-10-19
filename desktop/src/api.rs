//! The commands of the window: the only things it can ask of the app (`capabilities/default.json` grants
//! nothing else). All of them are thin; the work is in `core.rs`.

use std::sync::Arc;

use serde_json::Value;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::PendingRoute;
use crate::core::{ApiRequest, Core, Reply, SyncState};
use crate::store::OutboxRow;
use crate::tray::TrayLabels;

#[tauri::command]
pub async fn api(
    core: State<'_, Arc<Core>>,
    method: String,
    path: String,
    body: Option<Value>,
    queue: Option<bool>,
    idem_key: Option<String>,
) -> Result<Reply, String> {
    core.request(ApiRequest {
        method,
        path,
        body,
        queue: queue.unwrap_or(false),
        idem_key,
    })
    .await
}

#[tauri::command]
pub fn sync_state(core: State<'_, Arc<Core>>) -> SyncState {
    core.state()
}

#[tauri::command]
pub async fn sync_now(core: State<'_, Arc<Core>>) -> Result<(), String> {
    core.sync_now().await;
    Ok(())
}

#[tauri::command]
pub fn outbox_list(core: State<'_, Arc<Core>>) -> Result<Vec<OutboxRow>, String> {
    core.outbox()
}

#[tauri::command]
pub fn outbox_discard(core: State<'_, Arc<Core>>, id: i64) -> Result<(), String> {
    core.discard(id)
}

/// The window tells the app its language, for the notifications and the tray.
#[tauri::command]
pub fn set_language(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    language: String,
) -> Result<(), String> {
    if !matches!(language.as_str(), "en" | "uk") {
        return Err(format!("unknown language {language}"));
    }
    core.set_language(&language)?;
    if let Some(labels) = app.try_state::<TrayLabels>() {
        labels.speak(&language);
    }
    Ok(())
}

/// The screen a link from outside asked for, once.
#[tauri::command]
pub fn take_route(pending: State<'_, PendingRoute>) -> Option<String> {
    pending.0.lock().ok()?.take()
}

/// Starts sign-in with Google in the system browser (ADR-0062). The sign-in ends with a link back to the
/// app, which tells the window with the `google` event.
#[tauri::command]
pub async fn google_start(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    login_hint: Option<String>,
) -> Result<(), String> {
    let url = core.google_start(login_hint).await?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}
