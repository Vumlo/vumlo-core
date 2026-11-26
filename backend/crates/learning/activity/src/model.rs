//! What an activity type must provide, and the small data its signature names (ADR-0097,
//! ADR-0098). The contract is in tiers, and a type implements the tiers it has: every activity
//! is an [`ActivityType`]; one that has points and a gradebook column is also [`Graded`]; one a
//! student hands something in for is also [`Submissions`]; one that scores a submission by itself
//! is also [`AutoGrading`]. A type cannot claim a tier it does not implement.

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

/// What a type can do, as data the services and the SQL read instead of matching on the type. It
/// is derived from the tiers a type implements ([`ActivityType::capabilities`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// The activity has points, a config and a gradebook column: it is an assignment.
    pub graded: bool,
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

/// Every activity (ADR-0008, ADR-0097, ADR-0098). It is the future plugin boundary, so it stays
/// small.
pub trait ActivityType: Send + Sync {
    /// The text stored in the database for this type, for example `quiz`.
    fn id(&self) -> &'static str;

    /// What makes an activity of this type complete for a student.
    fn completion(&self) -> CompletionPolicy;

    /// The graded tier, or `None` for an activity that has no points. Every type answers, and
    /// a type that implements [`Graded`] returns `Some(self)`.
    fn graded(&self) -> Option<&dyn Graded>;

    /// What the type can do, derived from its tiers.
    fn capabilities(&self) -> Capabilities {
        let graded = self.graded();
        let submissions = graded.and_then(Graded::submissions);
        Capabilities {
            graded: graded.is_some(),
            submissions: submissions.is_some(),
            auto_graded: submissions.and_then(Submissions::auto_grading).is_some(),
        }
    }
}

/// An activity with points, a config and a gradebook column: an assignment.
pub trait Graded: Send + Sync {
    /// `Ok` when `config` is a valid configuration for this type, as the institution's editor
    /// profiles narrow what it may contain.
    fn validate_config(
        &self,
        config: &Map<String, Value>,
        profiles: &EffectiveProfiles,
    ) -> Result<(), ConfigError>;

    /// The submission tier, or `None` for a type nobody hands anything in for. Every graded type
    /// answers, and a type that implements [`Submissions`] returns `Some(self)`.
    fn submissions(&self) -> Option<&dyn Submissions>;

    /// What `max_points` must equal, for a type whose questions carry the points (ADR-0050).
    /// Default: the points are the teacher's own.
    fn points_total(&self, _config: &Map<String, Value>) -> Option<Decimal> {
        None
    }

    /// Removes from a config the facts the server adds when it reads one (ADR-0051 point 5).
    /// Default: the server adds none.
    fn strip_computed(&self, _config: &mut Map<String, Value>) {}

    /// The `config` as a student may read it: without the answer key (ADR-0023, ADR-0050).
    /// Default: the whole config, because it holds no key.
    fn student_config(&self, config: &Map<String, Value>) -> Map<String, Value> {
        config.clone()
    }

    /// The part of `config` that is frozen once work has been submitted, or `None` when none is
    /// (ADR-0023). Default: none.
    fn structure(&self, _config: &Map<String, Value>) -> Option<Value> {
        None
    }

    /// The files the documents of a `config` name (the questions of a quiz), each with its path in
    /// the config. A type that holds no document in its config names none (ADR-0069).
    fn config_file_refs(&self, _config: &Map<String, Value>) -> Result<Vec<FileRef>, ConfigError> {
        Ok(Vec::new())
    }
}

/// A graded activity a student hands something in for.
pub trait Submissions: Send + Sync {
    /// Checks the `content` of a draft or an attempt against the type, as narrowed by the
    /// institution's configuration (`profiles`) and by the assignment's own `config` (ADR-0025,
    /// ADR-0023).
    fn validate_submission(
        &self,
        config: &Map<String, Value>,
        content: &Value,
        stage: Stage,
        profiles: &EffectiveProfiles,
    ) -> Result<ValidContent, DocumentErrors>;

    /// What the content of an attempt claims from the rest of the course (ADR-0051).
    fn claims(&self, config: &Map<String, Value>, content: &Map<String, Value>) -> Claims;

    /// The auto-grading tier, or `None` for a type graded by hand. Every type that takes
    /// submissions answers, and one that implements [`AutoGrading`] returns `Some(self)`.
    fn auto_grading(&self) -> Option<&dyn AutoGrading>;

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

/// A type that scores a submitted attempt by itself (ADR-0050).
pub trait AutoGrading: Send + Sync {
    /// Scores a submitted attempt; `None` when the config cannot be read, so the attempt is left
    /// to a teacher.
    fn auto_grade(
        &self,
        config: &Map<String, Value>,
        content: &Map<String, Value>,
    ) -> Option<AutoGrade>;
}

#[cfg(test)]
mod tests;
