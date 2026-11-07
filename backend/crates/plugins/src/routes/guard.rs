//! What stands in front of the plugin API (ADR-0079): an allowance of calls for each token, and a
//! line in the log for each call. Neither reads a body.

use std::sync::Arc;
use std::time::{Duration, Instant};

use access::{AuthenticatedPlugin, PluginSource};
use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::KeyExtractor;
use tower_governor::{GovernorError, GovernorLayer};
use utoipa_axum::router::OpenApiRouter;
use vumlo_config::Settings;

use crate::PluginsError;
use crate::repo;
use crate::state::PluginsState;

/// How often the limiter forgets tokens whose allowance is full again. Without it the state grows
/// by one entry per token for as long as the server runs.
const CLEANUP_EVERY: Duration = Duration::from_secs(60);

/// The plugin a call belongs to, by its install id: the one `record` found for the token, which runs
/// before the limiter. A call with no good token counts against a single shared allowance, so
/// whoever sends garbage cannot make the limiter remember more than one entry (it has one entry
/// per install and this one), and cannot use up the allowance of a real plugin.
#[derive(Debug, Clone)]
pub struct InstallKey;

impl KeyExtractor for InstallKey {
    type Key = [u8; 16];

    fn extract<T>(&self, req: &axum::http::Request<T>) -> Result<Self::Key, GovernorError> {
        Ok(req
            .extensions()
            .get::<AuthenticatedPlugin>()
            .map_or([0; 16], |plugin| *plugin.install_id.as_bytes()))
    }
}

/// Puts the allowance of each token in front of `router`: a burst, refilled at a steady rate. The
/// limiter lives as long as the router does; a task forgets idle tokens.
pub fn limit<S>(
    router: OpenApiRouter<S>,
    settings: &Settings,
) -> Result<OpenApiRouter<S>, PluginsError>
where
    S: Clone + Send + Sync + 'static,
{
    let per_second = u64::from(settings.plugins.api_requests_per_second.max(1));
    let config = GovernorConfigBuilder::default()
        .per_millisecond((1000 / per_second).max(1))
        .burst_size(settings.plugins.api_burst.max(1))
        .key_extractor(InstallKey)
        .finish()
        .ok_or(PluginsError::RateLimit)?;
    let config = Arc::new(config);
    // The task holds only a weak reference: it ends when the router, and with it the limiter, goes.
    let limiter = Arc::downgrade(config.limiter());
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(CLEANUP_EVERY).await;
            let Some(limiter) = limiter.upgrade() else {
                break;
            };
            limiter.retain_recent();
            limiter.shrink_to_fit();
        }
    });
    Ok(router.layer(GovernorLayer::new(config)))
}

/// Finds the plugin behind the token once, leaves it for the limiter and the extractor of the handler
/// to use, and after the answer writes a line in the log: which plugin, which route (the template,
/// so no id), the answer and the time. A call whose token is not good is not logged: there is no
/// plugin to log it for, and the answer is the same `401` as ever. Neither is a call that was turned
/// away for its allowance: a flood must not also fill the log.
pub async fn record(State(state): State<PluginsState>, request: Request, next: Next) -> Response {
    let started = Instant::now();
    let (mut parts, body) = request.into_parts();
    let found = state.authenticate_plugin(&parts).await.ok();
    let route = parts.extensions.get::<MatchedPath>().map_or_else(
        || parts.uri.path().to_owned(),
        |path| path.as_str().to_owned(),
    );
    let method = parts.method.to_string();
    if let Some(plugin) = &found {
        parts
            .extensions
            .insert::<AuthenticatedPlugin>(plugin.clone());
    }
    let response = next.run(Request::from_parts(parts, body)).await;
    if let Some(plugin) = found.filter(|_| response.status().as_u16() != 429) {
        let elapsed = i32::try_from(started.elapsed().as_millis()).unwrap_or(i32::MAX);
        let status = i16::try_from(response.status().as_u16()).unwrap_or(i16::MAX);
        match state.db.acquire().await {
            Ok(mut conn) => {
                if let Err(error) = repo::insert_call(
                    &mut conn,
                    plugin.install_id,
                    &method,
                    &route,
                    status,
                    elapsed,
                )
                .await
                {
                    tracing::warn!(%error, "a plugin call was not logged");
                }
            }
            Err(error) => tracing::warn!(%error, "a plugin call was not logged"),
        }
    }
    response
}
