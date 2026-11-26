//! The `no_submission` type (ADR-0023): oral answers, class work, paper tests. Nothing is
//! submitted and it is graded by hand. Pure code: no database, no HTTP.

use serde_json::{Map, Value};

use crate::{ActivityType, CompletionPolicy, ConfigError, EffectiveProfiles, Graded, Submissions};

pub const ID: &str = "no_submission";

/// This type needs no settings; any field is a mistake.
pub fn validate_config(config: &Map<String, Value>) -> Result<(), ConfigError> {
    match config.keys().next() {
        None => Ok(()),
        Some(field) => Err(ConfigError::new(format!(
            "unknown field `{field}`: the {ID} type has no settings"
        ))),
    }
}

pub struct NoSubmission;

impl ActivityType for NoSubmission {
    fn id(&self) -> &'static str {
        ID
    }

    fn completion(&self) -> CompletionPolicy {
        CompletionPolicy::Graded
    }

    fn graded(&self) -> Option<&dyn Graded> {
        Some(self)
    }
}

impl Graded for NoSubmission {
    fn validate_config(
        &self,
        config: &Map<String, Value>,
        _profiles: &EffectiveProfiles,
    ) -> Result<(), ConfigError> {
        validate_config(config)
    }

    fn submissions(&self) -> Option<&dyn Submissions> {
        None
    }
}
