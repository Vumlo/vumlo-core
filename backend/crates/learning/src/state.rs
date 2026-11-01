use std::sync::Arc;

use sqlx::PgPool;
use vumlo_config::Settings;

use super::content::EffectiveProfiles;

/// What the handlers of `learning` need from the application.
#[derive(Clone)]
pub struct LearningState {
    pub db: PgPool,
    pub config: Arc<Settings>,
    /// The editor profiles, resolved once from `config.editor` (ADR-0025, ADR-0027).
    pub editor: Arc<EffectiveProfiles>,
    /// Where the bytes of uploaded files live (ADR-0069).
    pub files: files::Storage,
    /// `files.max_size`: the largest file the routes accept.
    pub max_file_bytes: u64,
}
