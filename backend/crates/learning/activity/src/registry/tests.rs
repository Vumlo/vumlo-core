//! The registry of activity types. The pure helpers on `Types` need no installed registry; the
//! free functions share one process-wide registry, installed once with both dummy types.

use serde_json::{Map, Value};

use super::*;
use crate::{
    AutoGrade, AutoGrading, Capabilities, Claims, CompletionPolicy, ConfigError, DocumentErrors,
    EffectiveProfiles, Graded, Stage, Submissions, ValidContent,
};

/// A type whose tiers are the ones its `capabilities` field says: a tier that is switched off
/// answers `None`, as a real type that does not implement it would.
struct Dummy {
    id: &'static str,
    capabilities: Capabilities,
}

impl ActivityType for Dummy {
    fn id(&self) -> &'static str {
        self.id
    }

    fn completion(&self) -> CompletionPolicy {
        CompletionPolicy::Graded
    }

    fn graded(&self) -> Option<&dyn Graded> {
        self.capabilities.graded.then_some(self as &dyn Graded)
    }
}

impl Graded for Dummy {
    fn validate_config(
        &self,
        _config: &Map<String, Value>,
        _profiles: &EffectiveProfiles,
    ) -> Result<(), ConfigError> {
        Ok(())
    }

    fn submissions(&self) -> Option<&dyn Submissions> {
        self.capabilities
            .submissions
            .then_some(self as &dyn Submissions)
    }
}

impl Submissions for Dummy {
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
        self.capabilities
            .auto_graded
            .then_some(self as &dyn AutoGrading)
    }
}

impl AutoGrading for Dummy {
    fn auto_grade(
        &self,
        _config: &Map<String, Value>,
        _content: &Map<String, Value>,
    ) -> Option<AutoGrade> {
        None
    }
}

/// Submits and is graded by itself.
static ZETA: Dummy = Dummy {
    id: "zeta",
    capabilities: Capabilities {
        graded: true,
        submissions: true,
        auto_graded: true,
    },
};

/// Is graded and takes no submission.
static ALPHA: Dummy = Dummy {
    id: "alpha_1",
    capabilities: Capabilities {
        graded: true,
        submissions: false,
        auto_graded: false,
    },
};

/// Has no points at all.
static PLAIN: Dummy = Dummy {
    id: "plain",
    capabilities: Capabilities {
        graded: false,
        submissions: false,
        auto_graded: false,
    },
};

fn both() -> Types {
    Types::new()
        .with(&ZETA)
        .and_then(|types| types.with(&ALPHA))
        .expect("two distinct valid ids")
}

#[test]
fn unknown_and_not_installed_are_errors() {
    let types = both();
    assert!(matches!(
        types.get("nope"),
        Err(RegistryError::Unknown(id)) if id == "nope"
    ));
    assert!(matches!(types.get(""), Err(RegistryError::Unknown(_))));
    assert_eq!(types.get("zeta").map(|kind| kind.id()), Ok("zeta"));
    // Nothing is installed in this test; an empty set answers `Unknown` too.
    assert!(matches!(
        Types::new().get("zeta"),
        Err(RegistryError::Unknown(_))
    ));
}

#[test]
fn duplicate_and_malformed_ids_are_refused() {
    assert!(matches!(
        both().with(&ZETA),
        Err(InstallError::DuplicateId(id)) if id == "zeta"
    ));
    for bad in [
        "",
        "Quiz",
        "1quiz",
        "_quiz",
        "qu-iz",
        "qu iz",
        "quizé",
        &"a".repeat(33),
    ] {
        let dummy: &'static Dummy = Box::leak(Box::new(Dummy {
            id: Box::leak(bad.to_owned().into_boxed_str()),
            capabilities: ZETA.capabilities,
        }));
        assert!(
            matches!(Types::new().with(dummy), Err(InstallError::BadId(id)) if id == bad),
            "`{bad}` must be refused"
        );
    }
    // The longest id and a digit or underscore after the first letter are fine.
    let longest: &'static Dummy = Box::leak(Box::new(Dummy {
        id: Box::leak("a".repeat(32).into_boxed_str()),
        capabilities: ZETA.capabilities,
    }));
    assert!(Types::new().with(longest).is_ok());
}

#[test]
fn ids_where_picks_by_capability() {
    let types = both();
    assert_eq!(types.ids_where(|c| c.submissions), ["zeta"]);
    assert_eq!(types.ids_where(|c| !c.submissions), ["alpha_1"]);
    assert_eq!(types.ids_where(|c| c.auto_graded), ["zeta"]);
    assert!(types.ids_where(|_| false).is_empty());
}

#[test]
fn available_is_sorted() {
    assert_eq!(both().available(), ["alpha_1", "zeta"]);
    assert!(Types::new().available().is_empty());
}

#[test]
fn kind_compares_by_id_and_derefs_to_the_trait() {
    let types = both();
    let zeta = types.get("zeta").expect("installed");
    assert_eq!(zeta, types.get("zeta").expect("installed"));
    assert_ne!(zeta, types.get("alpha_1").expect("installed"));
    assert_eq!(zeta.id(), "zeta");
    assert!(zeta.capabilities().auto_graded);
    assert_eq!(format!("{zeta:?}"), "Kind(\"zeta\")");
}

#[test]
fn kind_is_stored_as_text() {
    assert_eq!(
        <Kind as sqlx::Type<sqlx::Postgres>>::type_info(),
        <str as sqlx::Type<sqlx::Postgres>>::type_info()
    );
    assert_eq!(
        <GradedKind as sqlx::Type<sqlx::Postgres>>::type_info(),
        <str as sqlx::Type<sqlx::Postgres>>::type_info()
    );
}

fn three() -> Types {
    both().with(&PLAIN).expect("a third distinct id")
}

#[test]
fn the_capabilities_are_derived_from_the_tiers_a_type_returns() {
    let types = three();
    for dummy in [&ZETA, &ALPHA, &PLAIN] {
        let derived = types.get(dummy.id).expect("registered").capabilities();
        assert_eq!(derived, dummy.capabilities, "{}", dummy.id);
    }
    assert_eq!(types.ids_where(|c| c.graded), ["alpha_1", "zeta"]);
    assert_eq!(types.ids_where(|c| !c.graded), ["plain"]);
}

#[test]
fn only_a_graded_type_is_a_graded_kind() {
    let types = three();
    let alpha = types.get_graded("alpha_1").expect("graded");
    assert_eq!(alpha.id(), "alpha_1");
    assert_eq!(alpha.kind(), types.get("alpha_1").expect("registered"));
    assert_eq!(format!("{alpha:?}"), "GradedKind(\"alpha_1\")");
    assert!(alpha.submissions().is_none());
    let zeta = types.get_graded("zeta").expect("graded");
    assert!(zeta.submissions().is_some());
    assert_eq!(zeta.capabilities(), ZETA.capabilities);
    assert_eq!(zeta.completion(), CompletionPolicy::Graded);
    assert!(matches!(
        types.get_graded("plain"),
        Err(RegistryError::NotGraded(id)) if id == "plain"
    ));
    assert!(matches!(
        types.get_graded("nope"),
        Err(RegistryError::Unknown(id)) if id == "nope"
    ));
    assert!(types.get("plain").expect("registered").graded().is_none());
}

/// Without `testing` nothing installs a registry in this test binary, so it is the one place to
/// see the process that forgot to install one.
#[cfg(not(feature = "testing"))]
#[test]
fn not_installed_is_an_error_and_lists_are_empty() {
    assert!(matches!(get("zeta"), Err(RegistryError::NotInstalled)));
    assert!(available().is_empty());
    assert!(ids_where(|_| true).is_empty());
}

#[cfg(feature = "testing")]
mod global {
    use super::*;

    fn installed() {
        ensure_installed(both);
    }

    #[test]
    fn the_free_functions_read_the_installed_registry() {
        installed();
        assert_eq!(available(), ["alpha_1", "zeta"]);
        assert_eq!(ids_where(|c| c.submissions), ["zeta".to_owned()]);
        assert_eq!(get("alpha_1").map(|kind| kind.id()), Ok("alpha_1"));
        assert!(matches!(get("nope"), Err(RegistryError::Unknown(id)) if id == "nope"));
    }

    #[test]
    fn install_twice_fails() {
        installed();
        assert!(matches!(
            install(both()),
            Err(InstallError::AlreadyInstalled)
        ));
    }
}
