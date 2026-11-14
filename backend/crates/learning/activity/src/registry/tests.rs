//! The registry of activity types. The pure helpers on `Types` need no installed registry; the
//! free functions share one process-wide registry, installed once with both dummy types.

use serde_json::{Map, Value};
use sqlx::types::Decimal;

use super::*;
use crate::{
    AutoGrade, Capabilities, Claims, CompletionPolicy, ConfigError, DocumentErrors,
    EffectiveProfiles, Stage, ValidContent,
};

struct Dummy {
    id: &'static str,
    capabilities: Capabilities,
}

impl ActivityType for Dummy {
    fn id(&self) -> &'static str {
        self.id
    }

    fn capabilities(&self) -> Capabilities {
        self.capabilities
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

/// Submits and is graded by itself.
static ZETA: Dummy = Dummy {
    id: "zeta",
    capabilities: Capabilities {
        submissions: true,
        auto_graded: true,
    },
};

/// Takes no submission.
static ALPHA: Dummy = Dummy {
    id: "alpha_1",
    capabilities: Capabilities {
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
