//! Optional add-on modules (ADR-0054, ADR-0066): the check of `[modules]` against the specs of the
//! modules this binary was built with.
//!
//! A module has two switches. The Cargo feature of the same name compiles it in (without it the
//! name is unknown to this binary); `[modules] name = true` turns a compiled-in module on. Each
//! module declares its own [`ModuleSpec`]; nothing here lists modules.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::settings::Secret;

/// What the configuration needs to know about a module compiled into this binary. The module
/// declares it in its manifest (`module_api::Manifest::spec`).
#[derive(Debug, Clone, Copy)]
pub struct ModuleSpec {
    pub name: &'static str,
    /// Modules that must be enabled for this one to be.
    pub requires: &'static [&'static str],
    /// It has UI in the web app.
    pub web: bool,
    pub settings: &'static [SettingSpec],
}

#[derive(Debug, Clone, Copy)]
pub struct SettingSpec {
    pub key: &'static str,
    pub kind: SettingKind,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKind {
    Text,
    Bool,
    /// An `https://` URL without credentials (`http://` only for localhost with
    /// `allow_insecure_http = true`).
    Endpoint,
    /// Never in the file: read from this environment variable.
    Secret {
        env: &'static str,
    },
}

/// The keys every `[modules.name]` table has, besides the module's own.
pub const RESERVED_KEYS: &[&str] = &["enabled", "allow_insecure_http"];

/// The specs the tests of this crate use for the modules they call `dummy` and `dummy-dep`.
#[cfg(test)]
pub(crate) fn test_specs(names: &[&str]) -> Vec<ModuleSpec> {
    const ALL: &[ModuleSpec] = &[
        ModuleSpec {
            name: "dummy",
            requires: &[],
            web: true,
            settings: &[
                SettingSpec {
                    key: "endpoint",
                    kind: SettingKind::Endpoint,
                    required: false,
                },
                SettingSpec {
                    key: "api_key",
                    kind: SettingKind::Secret {
                        env: "VUMLO_DUMMY_API_KEY",
                    },
                    required: false,
                },
                SettingSpec {
                    key: "label",
                    kind: SettingKind::Text,
                    required: false,
                },
            ],
        },
        ModuleSpec {
            name: "dummy-dep",
            requires: &["dummy"],
            web: false,
            settings: &[],
        },
    ];
    ALL.iter()
        .filter(|m| names.contains(&m.name))
        .copied()
        .collect()
}

/// `name = true` or `[modules.name]` with `enabled` and the module's own keys. TOML cannot hold
/// both forms for one name; the table form is the one with settings.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ModuleEntry {
    Flag(bool),
    Table(BTreeMap<String, serde_json::Value>),
}

impl JsonSchema for ModuleEntry {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ModuleEntry".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        // Free-form: the registry, not the schema, knows each module's keys.
        schemars::json_schema!({"type": ["boolean", "object"], "x-free": true})
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ModuleStatus {
    /// Compiled in and on.
    Enabled,
    /// Compiled in and off.
    Disabled,
    /// Not compiled into this binary.
    Unavailable,
}

impl ModuleStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::Unavailable => "unavailable",
        }
    }
}

/// A module's settings after the check: public values and secrets kept apart.
#[derive(Debug, Clone, Default)]
pub struct ResolvedModule {
    pub enabled: bool,
    pub allow_insecure_http: bool,
    pub settings: BTreeMap<String, serde_json::Value>,
    pub secrets: BTreeMap<String, Secret>,
}

/// `[modules]` checked against the specs of the modules this binary was built with.
#[derive(Debug, Clone, Default)]
pub struct Modules {
    resolved: BTreeMap<String, ResolvedModule>,
    compiled: Vec<ModuleSpec>,
}

impl Modules {
    pub fn status(&self, name: &str) -> ModuleStatus {
        if !self.compiled.iter().any(|m| m.name == name) {
            ModuleStatus::Unavailable
        } else if self.is_enabled(name) {
            ModuleStatus::Enabled
        } else {
            ModuleStatus::Disabled
        }
    }

    pub fn is_enabled(&self, name: &str) -> bool {
        self.resolved.get(name).is_some_and(|m| m.enabled)
    }

    pub fn get(&self, name: &str) -> Option<&ResolvedModule> {
        self.resolved.get(name)
    }

    /// The specs of the modules compiled in, enabled or not.
    pub fn compiled(&self) -> &[ModuleSpec] {
        &self.compiled
    }

    /// The state of every module compiled in, by name.
    pub fn statuses(&self) -> BTreeMap<String, ModuleStatus> {
        self.compiled
            .iter()
            .map(|m| (m.name.to_owned(), self.status(m.name)))
            .collect()
    }
}

/// Checks `[modules]` and the module secrets in `env`; `compiled` holds the specs of the modules
/// this binary contains. Every problem is reported, not only the first.
pub fn resolve(
    entries: &BTreeMap<String, ModuleEntry>,
    env: &BTreeMap<String, String>,
    compiled: &[ModuleSpec],
) -> Result<Modules, Vec<String>> {
    let mut errors = Vec::new();
    let mut resolved = BTreeMap::new();
    for (name, entry) in entries {
        let Some(module) = compiled.iter().find(|m| m.name == name) else {
            let list = if compiled.is_empty() {
                "this build has no optional modules".to_owned()
            } else {
                let names: Vec<&str> = compiled.iter().map(|m| m.name).collect();
                format!("this build has: {}", names.join(", "))
            };
            errors.push(format!(
                "modules.{name}: unknown, or not compiled into this binary ({list}); a module is \
                 compiled in by the Cargo feature of its name"
            ));
            continue;
        };
        match resolve_one(module, entry, env) {
            Ok(one) => {
                resolved.insert(name.clone(), one);
            }
            Err(mut found) => errors.append(&mut found),
        }
    }
    for module in compiled {
        if !resolved.get(module.name).is_some_and(|m| m.enabled) {
            continue;
        }
        for required in module.requires {
            if !resolved.get(*required).is_some_and(|m| m.enabled) {
                errors.push(format!(
                    "modules.{}: needs modules.{required} to be enabled",
                    module.name
                ));
            }
        }
    }
    if errors.is_empty() {
        let mut compiled = compiled.to_vec();
        compiled.sort_by_key(|m| m.name);
        Ok(Modules { resolved, compiled })
    } else {
        Err(errors)
    }
}

fn resolve_one(
    module: &ModuleSpec,
    entry: &ModuleEntry,
    env: &BTreeMap<String, String>,
) -> Result<ResolvedModule, Vec<String>> {
    let name = module.name;
    let mut errors = Vec::new();
    let table = match entry {
        ModuleEntry::Flag(on) => {
            let one = ResolvedModule {
                enabled: *on,
                ..ResolvedModule::default()
            };
            return check_required(module, one, env);
        }
        ModuleEntry::Table(table) => table,
    };
    let mut one = ResolvedModule::default();
    for (key, value) in table {
        match key.as_str() {
            "enabled" => match value.as_bool() {
                Some(on) => one.enabled = on,
                None => errors.push(format!("modules.{name}.enabled must be true or false")),
            },
            "allow_insecure_http" => match value.as_bool() {
                Some(on) => one.allow_insecure_http = on,
                None => errors.push(format!(
                    "modules.{name}.allow_insecure_http must be true or false"
                )),
            },
            other => match module.settings.iter().find(|s| s.key == other) {
                None => errors.push(format!("modules.{name}.{other}: unknown setting")),
                Some(SettingSpec {
                    kind: SettingKind::Secret { env: var },
                    ..
                }) => errors.push(format!(
                    "modules.{name}.{other} is a secret: set {var} in the environment, not in \
                     vumlo.toml"
                )),
                Some(spec) => match value_ok(spec.kind, value) {
                    Ok(()) => {
                        one.settings.insert(other.to_owned(), value.clone());
                    }
                    Err(why) => errors.push(format!("modules.{name}.{other} {why}")),
                },
            },
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    // Endpoint rules depend on `allow_insecure_http`, so they run after the table is read.
    for spec in module.settings {
        if spec.kind == SettingKind::Endpoint
            && let Some(value) = one.settings.get(spec.key).and_then(|v| v.as_str())
            && let Err(why) = check_endpoint(value, one.allow_insecure_http)
        {
            errors.push(format!("modules.{name}.{} {why}", spec.key));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    check_required(module, one, env)
}

/// Reads the secrets and, for a module that is on, demands its required settings.
fn check_required(
    module: &ModuleSpec,
    mut one: ResolvedModule,
    env: &BTreeMap<String, String>,
) -> Result<ResolvedModule, Vec<String>> {
    let mut errors = Vec::new();
    for spec in module.settings {
        if let SettingKind::Secret { env: var } = spec.kind {
            match env.get(var).filter(|v| !v.is_empty()) {
                Some(value) => {
                    one.secrets
                        .insert(spec.key.to_owned(), Secret::new(value.as_str()));
                }
                None if one.enabled && spec.required => errors.push(format!(
                    "modules.{}: {var} is not set (the secret {} comes from the environment)",
                    module.name, spec.key
                )),
                None => {}
            }
        } else if one.enabled && spec.required && !one.settings.contains_key(spec.key) {
            errors.push(format!(
                "modules.{}.{} is required while the module is enabled",
                module.name, spec.key
            ));
        }
    }
    if errors.is_empty() {
        Ok(one)
    } else {
        Err(errors)
    }
}

fn value_ok(kind: SettingKind, value: &serde_json::Value) -> Result<(), &'static str> {
    match kind {
        SettingKind::Text | SettingKind::Endpoint if value.is_string() => Ok(()),
        SettingKind::Text | SettingKind::Endpoint => Err("must be a string"),
        SettingKind::Bool if value.is_boolean() => Ok(()),
        SettingKind::Bool => Err("must be true or false"),
        SettingKind::Secret { .. } => Err("is a secret"),
    }
}

/// `https://host[:port][/path]` without credentials. `http://` is accepted for `localhost`,
/// `127.0.0.1` and `[::1]` only, and only with the explicit flag.
pub fn check_endpoint(value: &str, allow_insecure_http: bool) -> Result<(), String> {
    let (scheme, rest) = value
        .split_once("://")
        .ok_or_else(|| format!("\"{value}\" must be an https:// URL"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() {
        return Err(format!("\"{value}\" has no host"));
    }
    if authority.contains('@') {
        return Err("must not contain credentials: put the key in the environment".to_owned());
    }
    if rest[authority.len()..].contains(['?', '#']) {
        return Err("must not carry a query or a fragment".to_owned());
    }
    let host = if let Some(inner) = authority.strip_prefix('[') {
        inner.split(']').next().unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    match scheme {
        "https" => Ok(()),
        "http" => {
            let local = matches!(host, "localhost" | "127.0.0.1" | "::1");
            if local && allow_insecure_http {
                Ok(())
            } else if local {
                Err(
                    "uses http://: set allow_insecure_http = true to allow it for localhost"
                        .to_owned(),
                )
            } else {
                Err("must use https:// (http:// is allowed for localhost only)".to_owned())
            }
        }
        _ => Err(format!("\"{value}\" must be an https:// URL")),
    }
}

#[cfg(test)]
mod tests;
