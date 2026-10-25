//! What the plugin API needs from the application: the pool, and the key that signs events.

use std::sync::Arc;

use access::{AuthenticatedPlugin, PluginSource};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use common::ApiError;
use sqlx::PgPool;
use vumlo_config::Settings;

use crate::model::PluginToken;
use crate::repo::{self, scopes_of};
use crate::secret::SecretKey;

#[derive(Clone)]
pub struct PluginsState {
    pub db: PgPool,
    /// `None` while `plugins.secret_key` is not set: no plugin is installed or sent anything then.
    pub key: Option<Arc<SecretKey>>,
}

impl PluginsState {
    pub fn new(db: PgPool, config: &Settings) -> Self {
        Self {
            db,
            key: SecretKey::from_config(&config.plugins.secret_key).map(Arc::new),
        }
    }
}

fn bearer(parts: &Parts) -> Option<&str> {
    parts
        .headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

impl PluginSource for PluginsState {
    /// The token names one enabled install. Only its SHA-256 is compared, by the database, so the
    /// token itself is never compared in a way that could leak it. Unknown, malformed, missing and
    /// disabled all give the same `401`.
    async fn authenticate_plugin(&self, parts: &Parts) -> Result<AuthenticatedPlugin, ApiError> {
        let denied = ApiError::Unauthorized("invalid or missing plugin token");
        let token = bearer(parts)
            .and_then(PluginToken::from_presented)
            .ok_or(denied)?;
        let mut conn = self
            .db
            .acquire()
            .await
            .map_err(|e| ApiError::from_db("plugins", &e))?;
        let owner = repo::find_enabled_by_token(&mut conn, &token.hash())
            .await
            .map_err(|e| ApiError::from_db("plugins", &e))?
            .ok_or(ApiError::Unauthorized("invalid or missing plugin token"))?;
        Ok(AuthenticatedPlugin {
            install_id: owner.install_id,
            plugin_id: owner.plugin_id,
            scopes: scopes_of(&owner.scopes),
        })
    }
}
