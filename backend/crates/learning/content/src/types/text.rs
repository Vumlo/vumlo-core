//! The `text` type (ADR-0025): the student writes one block document. Its config can only narrow
//! what the student's editor offers. Pure code: no database, no HTTP.

use std::collections::BTreeSet;

use api_types::{Feature, ProfileName};
use serde_json::{Map, Value};
use sqlx::types::Decimal;

use super::config_error;
use crate::model::ModelError;
use activity::{
    ActivityType, AutoGrade, Capabilities, Claims, CompletionPolicy, ConfigError, DocumentErrors,
    EffectiveProfiles, FileRef, Stage, ValidContent, file_refs_of, validate_document_within,
};

pub const ID: &str = "text";

/// The features a `text` assignment's `config.editor.features` allows, in the order given, or
/// `None` when the config does not narrow the profile. Only `{"editor": {"features": [..]}}` is a
/// valid config; the messages name the position of a mistake and never repeat what was sent.
pub fn editor_features(config: &Map<String, Value>) -> Result<Option<Vec<Feature>>, ModelError> {
    let invalid = |message: &str| Err(ModelError::Config(message.to_owned()));
    if let Some(field) = config.keys().find(|key| *key != "editor") {
        return Err(ModelError::Config(format!(
            "unknown field `{field}`: the text type has only `editor`"
        )));
    }
    let Some(editor) = config.get("editor") else {
        return Ok(None);
    };
    let Some(editor) = editor.as_object() else {
        return invalid("`editor` must be an object");
    };
    if editor.keys().any(|key| key != "features") {
        return invalid("unknown field in `editor`: it has only `features`");
    }
    let Some(features) = editor.get("features") else {
        return Ok(None);
    };
    let Some(features) = features.as_array() else {
        return invalid("`editor.features` must be a list of feature names");
    };
    let mut chosen = Vec::with_capacity(features.len());
    for (index, value) in features.iter().enumerate() {
        let Some(feature) = value.as_str().and_then(Feature::from_name) else {
            return Err(ModelError::Config(format!(
                "`editor.features[{index}]` is not a feature (expected one of: {})",
                Feature::ALL
                    .iter()
                    .map(|feature| feature.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        };
        if chosen.contains(&feature) {
            return Err(ModelError::Config(format!(
                "`editor.features[{index}]` is listed twice"
            )));
        }
        chosen.push(feature);
    }
    Ok(Some(chosen))
}

/// `config.editor.features` can only narrow the student profile (ADR-0025): a feature it lists
/// that the effective profile, after the institution's configuration, does not have is refused.
pub fn validate_config(
    config: &Map<String, Value>,
    profiles: &EffectiveProfiles,
) -> Result<(), ModelError> {
    let allowed = &profiles.profile(ProfileName::Student).features;
    for (index, feature) in editor_features(config)?.into_iter().flatten().enumerate() {
        if !allowed.contains(&feature) {
            return Err(ModelError::Config(format!(
                "`editor.features[{index}]` is not a feature of the student profile"
            )));
        }
    }
    Ok(())
}

/// Checks the content as one block document, narrowed by the config's own features.
pub fn validate_submission(
    config: &Map<String, Value>,
    content: &Value,
    profiles: &EffectiveProfiles,
) -> Result<ValidContent, DocumentErrors> {
    // A config that cannot be read narrows nothing further here: `validate_config` refused it
    // when it was written.
    let within: Option<BTreeSet<Feature>> = editor_features(config)
        .ok()
        .flatten()
        .map(|features| features.into_iter().collect());
    validate_document_within(content, ProfileName::Student, profiles, within.as_ref())
        .map(ValidContent::from)
}

/// The files the answer document names.
pub fn submission_file_refs(content: &Map<String, Value>) -> Result<Vec<FileRef>, DocumentErrors> {
    file_refs_of(&Value::Object(content.clone()))
}

pub struct Text;

impl ActivityType for Text {
    fn id(&self) -> &'static str {
        ID
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            submissions: true,
            auto_graded: false,
        }
    }

    fn completion(&self) -> CompletionPolicy {
        CompletionPolicy::SubmittedOrGraded
    }

    fn validate_config(
        &self,
        config: &Map<String, Value>,
        profiles: &EffectiveProfiles,
    ) -> Result<(), ConfigError> {
        validate_config(config, profiles).map_err(config_error)
    }

    fn points_total(&self, _config: &Map<String, Value>) -> Option<Decimal> {
        None
    }

    fn strip_computed(&self, _config: &mut Map<String, Value>) {}

    fn validate_submission(
        &self,
        config: &Map<String, Value>,
        content: &Value,
        _stage: Stage,
        profiles: &EffectiveProfiles,
    ) -> Result<ValidContent, DocumentErrors> {
        validate_submission(config, content, profiles)
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

    fn submission_file_refs(
        &self,
        _config: &Map<String, Value>,
        content: &Map<String, Value>,
    ) -> Result<Vec<FileRef>, DocumentErrors> {
        submission_file_refs(content)
    }
}
