//! `vumlo-cli version`.

use vumlo_config::SUPPORTED_CONFIG_VERSION;

use crate::runtime::{self, VERSION};

pub fn run() {
    println!("vumlo-cli {VERSION}");
    println!("config_version  {SUPPORTED_CONFIG_VERSION} (the newest this binary reads)");
    let latest = db::MIGRATOR
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
        .map(|m| m.version)
        .max();
    println!(
        "migrations      latest core migration {}",
        latest.map_or("none".to_owned(), |v| v.to_string())
    );
    let names = runtime::compiled_names();
    if names.is_empty() {
        println!("modules         none compiled in");
    } else {
        println!("modules         compiled in: {}", names.join(", "));
    }
}
