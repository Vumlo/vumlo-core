use std::sync::Arc;

use sqlx::PgPool;
use vumlo_config::{Language, MailSettings, Settings};

use crate::model::Limits;

/// What the routes and the callers of the mail layer need from the application.
#[derive(Clone)]
pub struct MailState {
    pub db: PgPool,
    /// `None` without `[mail]`: plugins cannot send (`503`), no worker starts.
    pub settings: Option<Arc<MailSettings>>,
    pub language: Language,
    pub institution_name: String,
    /// Where links in mails and in the answers of the opt-out routes point.
    pub link_base: String,
}

impl MailState {
    pub fn new(db: PgPool, config: &Settings) -> Self {
        let link_base = config
            .mail
            .as_ref()
            .and_then(|mail| mail.link_base.clone())
            .unwrap_or_else(|| config.server.public_origin.clone());
        Self {
            db,
            settings: config.mail.clone().map(Arc::new),
            language: config.institution.language,
            institution_name: config.institution.name.clone(),
            link_base,
        }
    }

    pub fn is_configured(&self) -> bool {
        self.settings.is_some()
    }

    pub fn limits(&self) -> Option<Limits> {
        self.settings.as_deref().map(Limits::from)
    }

    /// The most people one call may address; `None` without `[mail]`.
    pub fn max_audience(&self) -> Option<u32> {
        self.settings.as_deref().map(|mail| mail.max_audience)
    }
}
