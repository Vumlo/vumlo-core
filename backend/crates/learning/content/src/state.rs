use std::sync::Arc;

use sqlx::PgPool;

use crate::EffectiveProfiles;

/// What the handlers of `content` need from the application.
#[derive(Clone)]
pub struct ContentState {
    pub db: PgPool,
    /// The editor profiles, resolved once from `config.editor` (ADR-0025, ADR-0027).
    pub editor: Arc<EffectiveProfiles>,
}
