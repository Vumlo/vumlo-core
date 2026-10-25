//! What a module's handlers get: the pool, its settings, and a way to know who is calling.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use access::{Action, Actor, ActorSource, Authenticated};
use axum::http::request::Parts;
use common::ApiError;
use sqlx::PgPool;
use vumlo_config::ResolvedModule;

use crate::Manifest;

type RefFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// [`ActorSource`] without the `impl Future` return, so a module can hold the application's source
/// behind a pointer and never name its type.
pub trait DynActorSource: Send + Sync {
    fn authenticate<'a>(
        &'a self,
        parts: &'a Parts,
    ) -> RefFuture<'a, Result<Authenticated, ApiError>>;
}

impl<T: ActorSource> DynActorSource for T {
    fn authenticate<'a>(
        &'a self,
        parts: &'a Parts,
    ) -> RefFuture<'a, Result<Authenticated, ApiError>> {
        Box::pin(ActorSource::authenticate(self, parts))
    }
}

/// For the CLI, which has no requests: nobody is signed in.
pub struct NoActors;

impl DynActorSource for NoActors {
    fn authenticate<'a>(&'a self, _: &'a Parts) -> RefFuture<'a, Result<Authenticated, ApiError>> {
        Box::pin(async { Err(ApiError::Unauthorized("not signed in")) })
    }
}

/// The state of a module's router, handlers, jobs and subcommand. It implements [`ActorSource`], so
/// the `access::Actor` extractor works in a module's routes.
#[derive(Clone)]
pub struct ModuleState {
    pub db: PgPool,
    pub settings: Arc<ResolvedModule>,
    manifest: &'static Manifest,
    actors: Arc<dyn DynActorSource>,
}

impl ModuleState {
    pub fn new(
        manifest: &'static Manifest,
        db: PgPool,
        settings: ResolvedModule,
        actors: Arc<dyn DynActorSource>,
    ) -> Self {
        Self {
            db,
            settings: Arc::new(settings),
            manifest,
            actors,
        }
    }

    pub fn manifest(&self) -> &'static Manifest {
        self.manifest
    }

    /// Does `actor` hold the permission `name` that this module declared? A permission the module
    /// did not declare is held by nobody. `access` decides; this only finds the declaration.
    pub fn authorize(&self, actor: &Actor, name: &str) -> Result<(), ApiError> {
        let Some(permission) = self.manifest.permissions.iter().find(|p| p.name == name) else {
            return Err(ApiError::Forbidden("not allowed"));
        };
        access::authorize(
            actor,
            Action::Module {
                module: self.manifest.id,
                permission: permission.name,
                roles: permission.roles,
            },
        )?;
        Ok(())
    }
}

impl ActorSource for ModuleState {
    async fn authenticate(&self, parts: &Parts) -> Result<Authenticated, ApiError> {
        self.actors.authenticate(parts).await
    }
}
