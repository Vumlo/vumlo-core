//! What the plugin API needs from the application: the pool, and the key that signs events.

use std::sync::Arc;
use std::time::Duration;

use access::{AuthenticatedPlugin, PluginSource};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use common::ApiError;
use module_api::OutboundClient;
use sqlx::PgPool;
use vumlo_config::Settings;

use crate::limiter::RouteLimiter;
use crate::model::PluginToken;
use crate::repo::{self, course_scope, scopes_of};
use crate::secret::SecretKey;

#[derive(Clone)]
pub struct PluginsState {
    pub db: PgPool,
    /// `None` while `plugins.secret_key` is not set: no plugin is installed or sent anything then.
    pub key: Option<Arc<SecretKey>>,
    /// Carries the calls people make to a plugin's own server (ADR-0078). `None` if it could not be
    /// built, which answers those calls `503`.
    pub proxy: Option<Arc<OutboundClient>>,
    /// The allowance of each public route of a plugin (ADR-0080).
    pub public_routes: Arc<RouteLimiter>,
}

const PROXY_CONNECT: Duration = Duration::from_secs(3);
/// A call to a plugin's server that takes longer is a `504`.
const PROXY_TOTAL: Duration = Duration::from_secs(30);
/// The most a plugin's answer may hold.
pub const PROXY_MAX_RESPONSE_BYTES: usize = 20 * 1024 * 1024;

impl PluginsState {
    pub fn new(db: PgPool, config: &Settings) -> Self {
        Self {
            db,
            key: SecretKey::from_config(&config.plugins.secret_key).map(Arc::new),
            proxy: OutboundClient::new(PROXY_CONNECT, PROXY_TOTAL, PROXY_MAX_RESPONSE_BYTES, 0)
                .ok()
                .map(Arc::new),
            public_routes: Arc::new(RouteLimiter::new(
                config.plugins.public_route_burst,
                config.plugins.public_route_requests_per_second,
            )),
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
        // The guard in front of the plugin API (`routes::guard`) has looked the token up already.
        if let Some(found) = parts.extensions.get::<AuthenticatedPlugin>() {
            return Ok(found.clone());
        }
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
            courses: course_scope(owner.all_courses, owner.courses),
        })
    }
}
