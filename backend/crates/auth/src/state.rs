use std::sync::Arc;

use sqlx::PgPool;
use vumlo_config::Settings;

use super::{Google, HashGate};

/// What the handlers of `auth` need from the application.
#[derive(Clone)]
pub struct AuthState {
    pub db: PgPool,
    pub config: Arc<Settings>,
    /// The bound on password hashes and checks that run at once (ADR-0038).
    pub hashing: Arc<HashGate>,
    /// Sign-in with Google (ADR-0047); `None` without `[auth.google]`, and its endpoints are `404`.
    pub google: Option<Arc<Google>>,
}
