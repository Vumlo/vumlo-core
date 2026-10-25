//! The optional modules this binary was built with (ADR-0054, ADR-0066): one line per Cargo
//! feature, in [`compiled`]. This is the only place in the server that names a module; everything
//! else goes through [`module_api::Module`].

use std::sync::Arc;

use events::Handlers;
use module_api::{Module, ModuleState, OrderError, Registered, Registrar, init_order};
use sqlx::migrate::Migrator;
use vumlo_config::ModuleSpec;

use crate::app::AppState;

/// The modules compiled in, enabled or not, in no particular order.
pub fn compiled() -> Vec<&'static dyn Module> {
    vec![
        #[cfg(feature = "dummy")]
        vumlo_module_dummy::module(),
    ]
}

/// The modules compiled in, each after the ones it requires.
pub fn ordered() -> Result<Vec<&'static dyn Module>, OrderError> {
    init_order(&compiled())
}

pub fn compiled_names() -> Vec<&'static str> {
    compiled().iter().map(|m| m.manifest().id).collect()
}

/// What the configuration is checked against.
pub fn specs() -> Vec<ModuleSpec> {
    compiled().iter().map(|m| m.manifest().spec()).collect()
}

/// The migrators of the compiled-in modules: they run whether the module is enabled or not, so
/// switching a module on never needs a migration.
pub fn migrators() -> Vec<(&'static str, &'static Migrator)> {
    compiled()
        .iter()
        .filter_map(|m| m.migrations().map(|migrator| (m.manifest().id, migrator)))
        .collect()
}

/// A module that is compiled in and enabled, with what it registered.
pub(crate) struct Active {
    pub module: &'static dyn Module,
    pub state: ModuleState,
    pub registered: Registered,
}

/// Registers every enabled module, in start order.
pub(crate) fn active(app: &AppState) -> Vec<Active> {
    app.modules
        .iter()
        .filter(|module| app.config.module_state.is_enabled(module.manifest().id))
        .map(|module| {
            let id = module.manifest().id;
            let state = ModuleState::new(
                module.manifest(),
                app.db.clone(),
                app.config.module_state.get(id).cloned().unwrap_or_default(),
                Arc::new(app.clone()),
            );
            let mut registrar = Registrar::new(state.clone());
            module.register(&mut registrar);
            Active {
                module: *module,
                state,
                registered: registrar.finish(),
            }
        })
        .collect()
}

/// The event handlers of the enabled modules, for `events::run_worker`.
pub(crate) fn event_handlers(app: &AppState) -> Handlers {
    active(app)
        .into_iter()
        .fold(Handlers::default(), |all, one| {
            all.merge(one.registered.handlers)
        })
}

/// Starts the jobs of every module that is compiled in and enabled, and returns their ids. A
/// disabled module starts nothing.
pub fn spawn_workers(app: &AppState) -> Vec<&'static str> {
    let mut started = Vec::new();
    for one in active(app) {
        for job in one.registered.jobs {
            // The tasks run until the process ends; nothing waits for them.
            drop(job());
        }
        started.push(one.module.manifest().id);
    }
    started
}
