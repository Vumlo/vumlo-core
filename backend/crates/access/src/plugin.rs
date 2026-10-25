//! Who a plugin is when it calls the plugin API (ADR-0066), and what it may do. The counterpart of
//! [`Actor`](crate::Actor): only the extractor in this module builds a [`PluginCaller`], from what
//! [`PluginSource`] resolved, and [`authorize_plugin`] is where scopes are decided.

use std::future::Future;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use common::{ApiError, text_enum};
use uuid::Uuid;

use crate::AccessError;

/// What an administrator may grant a plugin. A scope is the whole of what one kind of call may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    /// Read a submitted attempt, never a draft.
    SubmissionsRead,
    /// Store the plugin's own report about a submitted attempt.
    ReportsWrite,
}
text_enum!(Scope { SubmissionsRead => "submissions:read", ReportsWrite => "reports:write" });

impl Scope {
    pub const ALL: [Scope; 2] = [Scope::SubmissionsRead, Scope::ReportsWrite];
}

/// An installed, enabled plugin whose token the request carried, with the scopes it was granted.
#[derive(Debug, Clone)]
pub struct PluginCaller {
    install_id: Uuid,
    plugin_id: String,
    scopes: Vec<Scope>,
}

impl PluginCaller {
    pub(crate) fn new(install_id: Uuid, plugin_id: String, scopes: Vec<Scope>) -> Self {
        Self {
            install_id,
            plugin_id,
            scopes,
        }
    }

    pub fn install_id(&self) -> Uuid {
        self.install_id
    }

    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }
}

/// A request's token, resolved by whoever holds the installs.
#[derive(Debug, Clone)]
pub struct AuthenticatedPlugin {
    pub install_id: Uuid,
    pub plugin_id: String,
    pub scopes: Vec<Scope>,
}

/// What the state of the application has to offer for a request to become a [`PluginCaller`]: it
/// finds the token, checks it and answers with the install. A missing, unknown or disabled plugin's
/// token is the error response, always `401`.
pub trait PluginSource: Send + Sync {
    fn authenticate_plugin(
        &self,
        parts: &Parts,
    ) -> impl Future<Output = Result<AuthenticatedPlugin, ApiError>> + Send;
}

impl<S: PluginSource> FromRequestParts<S> for PluginCaller {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let plugin = state.authenticate_plugin(parts).await?;
        Ok(Self::new(
            plugin.install_id,
            plugin.plugin_id,
            plugin.scopes,
        ))
    }
}

/// Allows `scope` when the plugin was granted it. Deny by default: no scope, no call.
pub fn authorize_plugin(caller: &PluginCaller, scope: Scope) -> Result<(), AccessError> {
    if caller.scopes.contains(&scope) {
        Ok(())
    } else {
        Err(AccessError::Denied)
    }
}

#[cfg(feature = "testing")]
impl PluginCaller {
    /// A caller with these scopes, for the tests of the crates above.
    pub fn for_tests(scopes: &[Scope]) -> Self {
        Self::for_install(Uuid::now_v7(), "test", scopes)
    }

    /// A caller that is the install `install_id`, for tests that go on to read its rows.
    pub fn for_install(install_id: Uuid, plugin_id: &str, scopes: &[Scope]) -> Self {
        Self::new(install_id, plugin_id.to_owned(), scopes.to_vec())
    }
}
