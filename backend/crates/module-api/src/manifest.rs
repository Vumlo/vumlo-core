//! What a module says about itself before anything runs.

use common::Role;
use vumlo_config::{ModuleSpec, SettingSpec};

/// A permission a module declares: a name and the institution roles that hold it. A route checks it
/// with [`ModuleState::authorize`](crate::ModuleState::authorize), which asks `access`.
#[derive(Debug, Clone, Copy)]
pub struct Permission {
    pub name: &'static str,
    pub roles: &'static [Role],
}

/// The static description of a module (ADR-0066). The settings are here, not in `register`,
/// because the configuration is checked before any module runs.
#[derive(Debug)]
pub struct Manifest {
    /// Lowercase letters, digits and `_`, starting with a letter: it is a path segment and, behind
    /// `mod_`, the name of the Postgres schema.
    pub id: &'static str,
    pub version: &'static str,
    /// Modules that must be enabled for this one to be.
    pub requires: &'static [&'static str],
    pub permissions: &'static [Permission],
    pub settings: &'static [SettingSpec],
    /// It has UI in the web app.
    pub web: bool,
}

impl Manifest {
    /// What `vumlo-config` needs to check `[modules]`.
    pub fn spec(&self) -> ModuleSpec {
        ModuleSpec {
            name: self.id,
            requires: self.requires,
            web: self.web,
            settings: self.settings,
        }
    }

    pub fn schema(&self) -> String {
        schema_of(self.id)
    }

    pub(crate) fn id_is_valid(&self) -> bool {
        let mut chars = self.id.chars();
        chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && self.id.len() <= 32
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    }
}

/// The Postgres schema of module `id`.
pub fn schema_of(id: &str) -> String {
    format!("mod_{id}")
}
