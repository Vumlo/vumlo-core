mod api;

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![api::api])
        .run(tauri::generate_context!())
        .expect("error while running the Vumlo desktop client");
}
