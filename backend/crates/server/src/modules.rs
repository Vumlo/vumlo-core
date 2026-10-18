//! The optional modules this binary was built with (ADR-0054): one line per Cargo feature. This
//! is the only place in the server that names a module; everything else goes through
//! [`module_api::ModuleDef`].

use module_api::{ModuleContext, ModuleDef};
use sqlx::migrate::Migrator;

use crate::app::AppState;

/// The modules compiled in, enabled or not.
pub fn compiled() -> Vec<ModuleDef> {
    vec![
        #[cfg(feature = "dummy")]
        vumlo_module_dummy::def(),
    ]
}

pub fn compiled_names() -> Vec<&'static str> {
    compiled().iter().map(|m| m.name).collect()
}

/// The migrators of the compiled-in modules: they run whether the module is enabled or not, so
/// switching a module on never needs a migration.
pub fn migrators() -> Vec<(&'static str, &'static Migrator)> {
    compiled()
        .iter()
        .filter_map(|m| m.migrator.map(|migrator| (m.name, migrator)))
        .collect()
}

pub(crate) fn context(state: &AppState, name: &str) -> ModuleContext {
    ModuleContext {
        db: state.db.clone(),
        settings: state
            .config
            .module_state
            .get(name)
            .cloned()
            .unwrap_or_default(),
    }
}

/// Starts the background work of every module that is compiled in and enabled, and returns their
/// names. A disabled module starts nothing.
pub fn spawn_workers(state: &AppState) -> Vec<&'static str> {
    let mut started = Vec::new();
    for module in compiled() {
        if state.config.module_state.is_enabled(module.name) {
            // The tasks run until the process ends; nothing waits for them.
            drop((module.workers)(context(state, module.name)));
            started.push(module.name);
        }
    }
    started
}
