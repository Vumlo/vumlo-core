fn main() {
    // The API address is compiled in from the client config that `vumlo-cli brand apply` wrote
    // (desktop/scripts/prepare.mjs exports it as VUMLO_API_URL). A build without it is a development build.
    println!("cargo:rerun-if-env-changed=VUMLO_API_URL");
    println!("cargo:rerun-if-env-changed=VUMLO_URL_SCHEME");
    if std::env::var_os("VUMLO_URL_SCHEME").is_none() {
        println!("cargo:rustc-env=VUMLO_URL_SCHEME=vumlo");
    }
    if std::env::var_os("VUMLO_API_URL").is_none() {
        println!("cargo:rustc-env=VUMLO_API_URL=http://127.0.0.1:8080/api/v1");
    }
    // `generate_context!` needs the frontend directory to exist, also for `cargo check` and `cargo test`.
    // A real build runs `npm run build:desktop` in web/ first, which fills it.
    std::fs::create_dir_all("../web/out").expect("web/out");
    tauri_build::build();
}
