//! The `no_submission` type (ADR-0023): oral answers, class work, paper tests. Nothing is
//! submitted and it is graded by hand. Pure code: no database, no HTTP.

use serde_json::{Map, Value};
use sqlx::types::Decimal;

use super::config_error;
use crate::model::ModelError;
use activity::{
    ActivityType, AutoGrade, Capabilities, Claims, CompletionPolicy, ConfigError, DocumentErrors,
    EffectiveProfiles, Stage, ValidContent,
};

pub const ID: &str = "no_submission";

/// This type needs no settings; any field is a mistake.
pub fn validate_config(config: &Map<String, Value>) -> Result<(), ModelError> {
    match config.keys().next() {
        None => Ok(()),
        Some(field) => Err(ModelError::Config(format!(
            "unknown field `{field}`: the {ID} type has no settings"
        ))),
    }
}

/// There is nothing to check: this type takes no submissions.
pub fn validate_submission() -> Result<ValidContent, DocumentErrors> {
    Err(DocumentErrors::one("", "this type takes no submissions"))
}

pub struct NoSubmission;

impl ActivityType for NoSubmission {
    fn id(&self) -> &'static str {
        ID
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            submissions: false,
            auto_graded: false,
        }
    }

    fn completion(&self) -> CompletionPolicy {
        CompletionPolicy::Graded
    }

    fn validate_config(
        &self,
        config: &Map<String, Value>,
        _profiles: &EffectiveProfiles,
    ) -> Result<(), ConfigError> {
        validate_config(config).map_err(config_error)
    }

    fn points_total(&self, _config: &Map<String, Value>) -> Option<Decimal> {
        None
    }

    fn strip_computed(&self, _config: &mut Map<String, Value>) {}

    fn validate_submission(
        &self,
        _config: &Map<String, Value>,
        _content: &Value,
        _stage: Stage,
        _profiles: &EffectiveProfiles,
    ) -> Result<ValidContent, DocumentErrors> {
        validate_submission()
    }

    fn student_config(&self, config: &Map<String, Value>) -> Map<String, Value> {
        config.clone()
    }

    fn structure(&self, _config: &Map<String, Value>) -> Option<Value> {
        None
    }

    fn auto_grade(
        &self,
        _config: &Map<String, Value>,
        _content: &Map<String, Value>,
    ) -> Option<AutoGrade> {
        None
    }

    fn claims(&self, _config: &Map<String, Value>, _content: &Map<String, Value>) -> Claims {
        Claims::default()
    }
}
