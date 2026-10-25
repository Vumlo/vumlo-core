use std::sync::Arc;

use sqlx::PgPool;
use vumlo_config::Settings;

/// What the handlers of `org` need from the application.
#[derive(Clone)]
pub struct OrgState {
    pub db: PgPool,
    pub config: Arc<Settings>,
}
