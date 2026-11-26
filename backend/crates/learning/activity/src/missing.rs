//! The stand-in for a stored type nobody registered (ADR-0098, decision D4). A module that is
//! switched off, or removed from the build, leaves rows whose `type` this process does not know.
//! Reading them must not fail the course tree around them, so a stored id that is not in the
//! registry decodes to a [`Missing`] type: it has no behaviour, its body is never shown, and
//! nothing can be written to it. `doctor` names the ids.

use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex, PoisonError};

use serde_json::{Map, Value};

use crate::{
    ActivityType, CompletionPolicy, ConfigError, DocumentErrors, EffectiveProfiles, FileRef,
    Graded, Submissions,
};

/// The type of a stored id that is not registered. One value per id and per tier, made once and
/// kept for the life of the process: a stored id is `^[a-z][a-z0-9_]{0,31}$` (a CHECK of the
/// database), so the set is small.
pub(crate) struct Missing {
    id: &'static str,
    /// Whether the row it stands for is an assignment: its points and scores are still read.
    graded: bool,
}

static MADE: LazyLock<Mutex<BTreeMap<(String, bool), &'static Missing>>> =
    LazyLock::new(Mutex::default);

impl Missing {
    pub(crate) fn of(id: &str, graded: bool) -> &'static Missing {
        let mut made = MADE.lock().unwrap_or_else(PoisonError::into_inner);
        made.entry((id.to_owned(), graded)).or_insert_with(|| {
            // Leaked on purpose, once per distinct id: the `'static` id is the contract of `Kind`.
            Box::leak(Box::new(Missing {
                id: Box::leak(id.to_owned().into_boxed_str()),
                graded,
            }))
        })
    }
}

const UNAVAILABLE: &str = "the module that provides this activity type is not enabled";

impl ActivityType for Missing {
    fn id(&self) -> &'static str {
        self.id
    }

    /// Only a grade completes it: a student cannot mark an item they cannot see.
    fn completion(&self) -> CompletionPolicy {
        CompletionPolicy::Graded
    }

    fn graded(&self) -> Option<&dyn Graded> {
        self.graded.then_some(self as &dyn Graded)
    }

    fn available(&self) -> bool {
        false
    }

    fn default_body(&self) -> Map<String, Value> {
        Map::new()
    }

    fn validate_body(
        &self,
        _body: &Value,
        _profiles: &EffectiveProfiles,
    ) -> Result<Map<String, Value>, DocumentErrors> {
        Err(DocumentErrors::one("", UNAVAILABLE))
    }

    /// Names nothing: the body of a type nobody can read is not parsed.
    fn body_file_refs(&self, _body: &Map<String, Value>) -> Result<Vec<FileRef>, DocumentErrors> {
        Ok(Vec::new())
    }
}

impl Graded for Missing {
    fn validate_config(
        &self,
        _config: &Map<String, Value>,
        _profiles: &EffectiveProfiles,
    ) -> Result<(), ConfigError> {
        Err(ConfigError::new(UNAVAILABLE))
    }

    fn submissions(&self) -> Option<&dyn Submissions> {
        None
    }
}
