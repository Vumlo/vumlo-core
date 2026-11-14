//! What an activity type must provide, and the small data its signature names (ADR-0097).

use std::collections::BTreeMap;

use serde_json::{Map, Value};
use sqlx::types::Decimal;
use uuid::Uuid;

use crate::{AutoGrade, Claims, DocumentErrors, EffectiveProfiles, FileRef, Stage, ValidContent};

/// A configuration the type refuses. The text is fixed and never repeats what was sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ConfigError(String);

impl ConfigError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// What a type can do, as data the services read instead of matching on the type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// A student submits something for this type. Only such a type has the deadline settings of
    /// a submission (ADR-0035).
    pub submissions: bool,
    /// The type scores a submitted attempt by itself (ADR-0050).
    pub auto_graded: bool,
}

/// Where a student stands on one activity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    pub submitted: bool,
    pub graded: bool,
    /// The student marked it done by hand. No policy uses it: marking is the item's rule, not a
    /// type's.
    pub marked: bool,
}

/// What makes an activity complete for a type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionPolicy {
    /// A submission or a grade completes it.
    SubmittedOrGraded,
    /// Only a grade completes it.
    Graded,
}

impl CompletionPolicy {
    pub fn is_met(self, progress: Progress) -> bool {
        match self {
            Self::SubmittedOrGraded => progress.submitted || progress.graded,
            Self::Graded => progress.graded,
        }
    }
}

/// What an activity type must provide (ADR-0008, ADR-0097). It is the future plugin boundary, so
/// it stays small.
pub trait ActivityType: Send + Sync {
    /// The text stored in the database for this type, for example `quiz`.
    fn id(&self) -> &'static str;

    /// What the type can do.
    fn capabilities(&self) -> Capabilities;

    /// What makes an activity of this type complete for a student.
    fn completion(&self) -> CompletionPolicy;

    /// `Ok` when `config` is a valid configuration for this type, as the institution's editor
    /// profiles narrow what it may contain.
    fn validate_config(
        &self,
        config: &Map<String, Value>,
        profiles: &EffectiveProfiles,
    ) -> Result<(), ConfigError>;

    /// What `max_points` must equal, for a type whose questions carry the points (ADR-0050).
    fn points_total(&self, config: &Map<String, Value>) -> Option<Decimal>;

    /// Removes from a config the facts the server adds when it reads one (ADR-0051 point 5).
    fn strip_computed(&self, config: &mut Map<String, Value>);

    /// Checks the `content` of a draft or an attempt against the type, as narrowed by the
    /// institution's configuration (`profiles`) and by the assignment's own `config` (ADR-0025,
    /// ADR-0023). It is called only for a type whose capabilities take submissions.
    fn validate_submission(
        &self,
        config: &Map<String, Value>,
        content: &Value,
        stage: Stage,
        profiles: &EffectiveProfiles,
    ) -> Result<ValidContent, DocumentErrors>;

    /// The `config` as a student may read it: without the answer key (ADR-0023, ADR-0050).
    fn student_config(&self, config: &Map<String, Value>) -> Map<String, Value>;

    /// The part of `config` that is frozen once work has been submitted, or `None` when none is
    /// (ADR-0023).
    fn structure(&self, config: &Map<String, Value>) -> Option<Value>;

    /// Scores a submitted attempt by itself; `None` for a type graded by hand (ADR-0050).
    fn auto_grade(
        &self,
        config: &Map<String, Value>,
        content: &Map<String, Value>,
    ) -> Option<AutoGrade>;

    /// What the content of an attempt claims from the rest of the course (ADR-0051).
    fn claims(&self, config: &Map<String, Value>, content: &Map<String, Value>) -> Claims;

    /// The files the documents of a `config` name (the questions of a quiz), each with its path in
    /// the config. A type that holds no document in its config names none (ADR-0069).
    fn config_file_refs(&self, _config: &Map<String, Value>) -> Result<Vec<FileRef>, ConfigError> {
        Ok(Vec::new())
    }

    /// Every file a stored `content` names, each with its path in the content. A type that holds
    /// no document in its content names none (ADR-0069).
    fn submission_file_refs(
        &self,
        _config: &Map<String, Value>,
        _content: &Map<String, Value>,
    ) -> Result<Vec<FileRef>, DocumentErrors> {
        Ok(Vec::new())
    }

    /// Checks the names of the files of a content against what the type accepts. `names` maps a
    /// file id to its name; the caller resolves them, since the type is pure (ADR-0069).
    fn check_file_names(
        &self,
        _config: &Map<String, Value>,
        _content: &Map<String, Value>,
        _names: &BTreeMap<Uuid, String>,
    ) -> Result<(), DocumentErrors> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
