mod api;
mod core;
mod deeplink;
mod keychain;
mod notify;
mod store;
mod tray;

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_notification::NotificationExt;

use crate::core::Core;
use crate::keychain::KeychainToken;
use crate::store::Store;

const API_URL: &str = env!("VUMLO_API_URL");

/// The screen a link from outside asked for, until the window takes it (`take_route`).
#[derive(Default)]
pub struct PendingRoute(pub Mutex<Option<String>>);

/// Whether closing the window hides it (the tray can bring it back) instead of quitting.
#[derive(Default)]
pub struct HideOnClose(pub AtomicBool);

fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn open_link(app: &AppHandle, link: &str) {
    if let Some(code) = deeplink::google_code(link, deeplink::scheme()) {
        // The end of a sign-in with Google (ADR-0062): trade the code, then tell the window.
        show(app);
        let (app, core) = (app.clone(), app.state::<Arc<Core>>().inner().clone());
        tauri::async_runtime::spawn(async move {
            let outcome = match core.google_finish(&code).await {
                Ok(user) => serde_json::json!({"ok": true, "user": user}),
                Err(error) => serde_json::json!({"ok": false, "error": error}),
            };
            let _ = app.emit("google", outcome);
        });
        return;
    }
    let Some(route) = deeplink::route_of(link, deeplink::scheme()) else {
        return;
    };
    if let Ok(mut pending) = app.state::<PendingRoute>().0.lock() {
        *pending = Some(route);
    }
    show(app);
    let _ = app.emit("navigate", ());
}

pub fn run() {
    tauri::Builder::default()
        // First, so a second launch (or a link) reaches the running app before anything else starts.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show(app)
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let store = Store::open(&dir.join("offline.sqlite3"))?;
            let window = app.handle().clone();
            let core = Arc::new(Core::new(
                store,
                API_URL.to_owned(),
                Arc::new(KeychainToken::new(&app.config().identifier)),
                Box::new(move |state| {
                    let _ = window.emit("sync", state);
                }),
            ));
            app.manage(core.clone());
            app.manage(PendingRoute::default());
            tauri::async_runtime::spawn(core.clone().run_outbox());

            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    open_link(&handle, url.as_str());
                }
            });
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                for url in urls {
                    open_link(app.handle(), url.as_str());
                }
            }
            // An installed app is registered for its links by its installer; a build run by hand is not.
            #[cfg(debug_assertions)]
            let _ = app.deep_link().register_all();

            let hide = tray::build(app.handle()).is_ok();
            app.manage(HideOnClose(AtomicBool::new(hide)));

            let _ = app.notification().request_permission();
            tauri::async_runtime::spawn(notify::run(core, app.handle().clone()));
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && window
                    .app_handle()
                    .state::<HideOnClose>()
                    .0
                    .load(Ordering::Relaxed)
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            api::api,
            api::sync_state,
            api::sync_now,
            api::outbox_list,
            api::outbox_discard,
            api::set_language,
            api::take_route,
            api::google_start,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Vumlo desktop client");
}
