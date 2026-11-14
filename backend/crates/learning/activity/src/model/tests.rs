//! The data of the contract and the defaults of the trait.

use std::collections::BTreeMap;

use serde_json::{Map, Value};
use sqlx::types::Decimal;
use uuid::Uuid;

use super::*;
use crate::{AutoGrade, Claims, EffectiveProfiles, Stage, ValidContent};

#[test]
fn completion_policy_table() {
    for bits in 0..8u8 {
        let progress = Progress {
            submitted: bits & 1 != 0,
            graded: bits & 2 != 0,
            marked: bits & 4 != 0,
        };
        assert_eq!(
            CompletionPolicy::SubmittedOrGraded.is_met(progress),
            progress.submitted || progress.graded,
            "{progress:?}"
        );
        assert_eq!(
            CompletionPolicy::Graded.is_met(progress),
            progress.graded,
            "{progress:?}"
        );
    }
}

#[test]
fn marking_alone_meets_no_policy() {
    let marked = Progress {
        marked: true,
        ..Progress::default()
    };
    assert!(!CompletionPolicy::SubmittedOrGraded.is_met(marked));
    assert!(!CompletionPolicy::Graded.is_met(marked));
}

#[test]
fn a_config_error_displays_its_text() {
    assert_eq!(ConfigError::new("x").to_string(), "x");
}

/// Implements only what has no default.
struct Minimal;

impl ActivityType for Minimal {
    fn id(&self) -> &'static str {
        "minimal"
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
        _config: &Map<String, Value>,
        _profiles: &EffectiveProfiles,
    ) -> Result<(), ConfigError> {
        Ok(())
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
        Ok(ValidContent::new(Map::new(), true))
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

static MINIMAL: Minimal = Minimal;

#[test]
fn the_file_hooks_default_to_nothing_and_the_trait_is_object_safe() {
    let kind: &'static dyn ActivityType = &MINIMAL;
    let none = Map::new();
    assert_eq!(kind.id(), "minimal");
    assert_eq!(kind.config_file_refs(&none), Ok(Vec::new()));
    assert_eq!(kind.submission_file_refs(&none, &none), Ok(Vec::new()));
    assert_eq!(
        kind.check_file_names(&none, &none, &BTreeMap::<Uuid, String>::new()),
        Ok(())
    );
}
