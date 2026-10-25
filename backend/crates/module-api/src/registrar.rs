//! What a module adds to the application in `register`.

use events::{HandlerFuture, Handlers, OutboxEvent};
use tokio::task::JoinHandle;
use utoipa_axum::router::OpenApiRouter;

use crate::ModuleState;

/// Background work: started by the worker (and by `serve`, which runs it too) for an enabled module.
pub type Job = Box<dyn FnOnce() -> JoinHandle<()> + Send>;

/// Collects the routes, event handlers and jobs of one module.
pub struct Registrar {
    state: ModuleState,
    routes: OpenApiRouter<ModuleState>,
    handlers: Handlers,
    jobs: Vec<Job>,
}

/// What a module registered.
pub struct Registered {
    pub routes: OpenApiRouter<ModuleState>,
    pub handlers: Handlers,
    pub jobs: Vec<Job>,
}

impl Registrar {
    pub fn new(state: ModuleState) -> Self {
        Self {
            state,
            routes: OpenApiRouter::new(),
            handlers: Handlers::default(),
            jobs: Vec::new(),
        }
    }

    /// The module's own state, for building values that need its settings.
    pub fn state(&self) -> &ModuleState {
        &self.state
    }

    /// Routes relative to `/api/v1/modules/<id>`. Their state is the [`ModuleState`].
    pub fn routes(&mut self, routes: OpenApiRouter<ModuleState>) {
        let current = std::mem::take(&mut self.routes);
        self.routes = current.merge(routes);
    }

    /// Runs `handler` for every outbox event of type `kind` (ADR-0045), after the core's own
    /// handlers. Delivery is at least once: the handler tolerates a repeat.
    pub fn on(
        &mut self,
        kind: &'static str,
        handler: impl Fn(ModuleState, OutboxEvent) -> HandlerFuture + Send + Sync + 'static,
    ) {
        let state = self.state.clone();
        let handlers = std::mem::take(&mut self.handlers);
        self.handlers = handlers.on(kind, move |event| handler(state.clone(), event));
    }

    /// Work that runs until the process ends.
    pub fn job(&mut self, start: impl FnOnce(ModuleState) -> JoinHandle<()> + Send + 'static) {
        let state = self.state.clone();
        self.jobs.push(Box::new(move || start(state)));
    }

    pub fn finish(self) -> Registered {
        Registered {
            routes: self.routes,
            handlers: self.handlers,
            jobs: self.jobs,
        }
    }
}
