//! The data of the contract and the defaults of the traits.

use std::collections::BTreeMap;

use serde_json::{Map, Value};
use uuid::Uuid;

use super::*;
use crate::{Claims, EffectiveProfiles, Stage, ValidContent};

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

/// Implements only what has no default: the three tiers down to a submission that nothing scores.
struct Minimal;

impl ActivityType for Minimal {
    fn id(&self) -> &'static str {
        "minimal"
    }

    fn completion(&self) -> CompletionPolicy {
        CompletionPolicy::Graded
    }

    fn graded(&self) -> Option<&dyn Graded> {
        Some(self)
    }
}

impl Graded for Minimal {
    fn validate_config(
        &self,
        _config: &Map<String, Value>,
        _profiles: &EffectiveProfiles,
    ) -> Result<(), ConfigError> {
        Ok(())
    }

    fn submissions(&self) -> Option<&dyn Submissions> {
        Some(self)
    }
}

impl Submissions for Minimal {
    fn validate_submission(
        &self,
        _config: &Map<String, Value>,
        _content: &Value,
        _stage: Stage,
        _profiles: &EffectiveProfiles,
    ) -> Result<ValidContent, DocumentErrors> {
        Ok(ValidContent::new(Map::new(), true))
    }

    fn claims(&self, _config: &Map<String, Value>, _content: &Map<String, Value>) -> Claims {
        Claims::default()
    }

    fn auto_grading(&self) -> Option<&dyn AutoGrading> {
        None
    }
}

static MINIMAL: Minimal = Minimal;

#[test]
fn the_hooks_default_to_nothing_and_the_traits_are_object_safe() {
    let kind: &'static dyn ActivityType = &MINIMAL;
    let graded = kind.graded().expect("a graded type");
    let submissions = graded.submissions().expect("a type with submissions");
    let none = Map::new();
    assert_eq!(kind.id(), "minimal");
    assert_eq!(graded.points_total(&none), None);
    assert_eq!(graded.student_config(&none), none);
    assert_eq!(graded.structure(&none), None);
    assert_eq!(graded.config_file_refs(&none), Ok(Vec::new()));
    assert_eq!(
        submissions.submission_file_refs(&none, &none),
        Ok(Vec::new())
    );
    assert_eq!(
        submissions.check_file_names(&none, &none, &BTreeMap::<Uuid, String>::new()),
        Ok(())
    );
    let mut config = Map::new();
    config.insert("k".to_owned(), Value::Null);
    graded.strip_computed(&mut config);
    assert_eq!(config.len(), 1);
}

#[test]
fn the_capabilities_follow_the_tiers_a_type_has() {
    assert_eq!(
        MINIMAL.capabilities(),
        Capabilities {
            graded: true,
            submissions: true,
            auto_graded: false,
        }
    );
}
