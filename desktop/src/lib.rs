mod api;
mod core;
mod store;

use std::sync::Arc;

use tauri::{Emitter, Manager};

use crate::core::{Core, MemoryToken};
use crate::store::Store;

const API_URL: &str = env!("VUMLO_API_URL");

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let store = Store::open(&dir.join("offline.sqlite3"))?;
            let window = app.handle().clone();
            let core = Arc::new(Core::new(
                store,
                API_URL.to_owned(),
                Arc::new(MemoryToken::default()),
                Box::new(move |state| {
                    let _ = window.emit("sync", state);
                }),
            ));
            app.manage(core.clone());
            tauri::async_runtime::spawn(core.run_outbox());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            api::api,
            api::sync_state,
            api::sync_now,
            api::outbox_list,
            api::outbox_discard,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Vumlo desktop client");
}
