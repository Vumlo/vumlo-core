//! The JSON Schema of [`Settings`] and what is read from it: which keys exist, and which are
//! public, private or secret.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use schemars::SchemaGenerator;
use schemars::generate::SchemaSettings;
use serde_json::Value;

use crate::settings::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Private,
    Secret,
}

impl Visibility {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "public" => Some(Self::Public),
            "private" => Some(Self::Private),
            "secret" => Some(Self::Secret),
            _ => None,
        }
    }
}

/// One key of the schema. `visibility` is the effective one: the key's own mark, else the nearest
/// section's. `*` stands for the name of a map entry.
#[derive(Debug, Clone)]
pub struct Key {
    pub path: String,
    pub visibility: Option<Visibility>,
    pub leaf: bool,
}

/// The schema of `vumlo.toml`, subschemas inlined.
pub fn settings_schema() -> &'static Value {
    static SCHEMA: LazyLock<Value> = LazyLock::new(|| {
        let mut settings = SchemaSettings::draft2020_12();
        settings.inline_subschemas = true;
        schema_of::<Settings>(settings.into_generator())
    });
    &SCHEMA
}

pub fn schema_of<T: schemars::JsonSchema>(mut generator: SchemaGenerator) -> Value {
    serde_json::to_value(generator.root_schema_for::<T>()).unwrap_or(Value::Null)
}

/// Every key of the schema, depth first.
pub fn keys() -> &'static [Key] {
    static KEYS: LazyLock<Vec<Key>> = LazyLock::new(|| {
        let mut found = Vec::new();
        collect(settings_schema(), "", None, &mut found);
        found
    });
    &KEYS
}

fn own_visibility(node: &Value) -> Option<Visibility> {
    let mark = |n: &Value| {
        n.get("x-visibility")
            .and_then(Value::as_str)
            .and_then(Visibility::parse)
    };
    mark(node).or_else(|| members(node).find_map(mark))
}

/// The alternatives of a node (`anyOf`, `oneOf`, `allOf`).
fn members(node: &Value) -> impl Iterator<Item = &Value> {
    ["anyOf", "oneOf", "allOf"]
        .into_iter()
        .filter_map(|k| node.get(k).and_then(Value::as_array))
        .flatten()
}

/// The named children of a node, looking through its alternatives. A map's entries are `*`.
fn children(node: &Value) -> Vec<(String, &Value)> {
    let mut found = Vec::new();
    if let Some(props) = node.get("properties").and_then(Value::as_object) {
        found.extend(props.iter().map(|(k, v)| (k.clone(), v)));
    }
    if let Some(extra) = node.get("additionalProperties").filter(|v| v.is_object()) {
        found.push(("*".to_owned(), extra));
    }
    for member in members(node) {
        found.extend(children(member));
    }
    found
}

fn collect(node: &Value, path: &str, inherited: Option<Visibility>, out: &mut Vec<Key>) {
    let visibility = own_visibility(node).or(inherited);
    let kids = children(node);
    if !path.is_empty() {
        let free = node.get("x-free").and_then(Value::as_bool) == Some(true);
        out.push(Key {
            path: path.to_owned(),
            visibility,
            leaf: kids.is_empty(),
        });
        if free {
            return;
        }
    }
    for (name, child) in kids {
        let child_path = if path.is_empty() {
            name
        } else {
            format!("{path}.{name}")
        };
        collect(child, &child_path, visibility, out);
    }
}

/// Paths of the keys that are secrets: `database.url`, ...
pub fn secret_paths() -> BTreeSet<&'static str> {
    keys()
        .iter()
        .filter(|k| k.visibility == Some(Visibility::Secret) && k.leaf)
        .map(|k| k.path.as_str())
        .collect()
}

/// Paths of the keys that may reach the browser.
pub fn public_paths() -> Vec<&'static str> {
    keys()
        .iter()
        .filter(|k| k.visibility == Some(Visibility::Public) && k.leaf)
        .map(|k| k.path.as_str())
        .collect()
}

/// The leaf keys that have no visibility, by path. Must be empty.
#[cfg(test)]
pub fn unmarked() -> Vec<&'static str> {
    keys()
        .iter()
        .filter(|k| k.leaf && k.visibility.is_none())
        .map(|k| k.path.as_str())
        .collect()
}

/// The environment variable a secret path is read from.
pub fn env_name(path: &str) -> String {
    if path == "database.url" {
        "DATABASE_URL".to_owned()
    } else {
        format!("VUMLO__{}", path.replace('.', "__").to_uppercase())
    }
}

/// Paths in `value` that the schema does not know, each as `a.b.c`.
pub fn unknown_keys(value: &Value) -> Vec<String> {
    let mut found = Vec::new();
    unknown_in(settings_schema(), value, "", &mut found);
    found
}

fn unknown_in(node: &Value, value: &Value, path: &str, out: &mut Vec<String>) {
    if node.get("x-free").and_then(Value::as_bool) == Some(true) {
        return;
    }
    let Some(object) = value.as_object() else {
        return;
    };
    let kids = children(node);
    for (key, child_value) in object {
        let child_path = if path.is_empty() {
            key.clone()
        } else {
            format!("{path}.{key}")
        };
        let mut matches = kids
            .iter()
            .filter(|(name, _)| name == key || name == "*")
            .peekable();
        if matches.peek().is_none() {
            out.push(child_path);
            continue;
        }
        for (_, child) in matches {
            unknown_in(child, child_value, &child_path, out);
        }
    }
}

/// Words that mark a key as holding a secret, for keys the schema does not know.
const SECRET_WORDS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "private_key",
    "signing_key",
    "encryption_key",
    "credential",
];

pub fn looks_secret(path: &str) -> bool {
    let last = path.rsplit('.').next().unwrap_or(path).to_lowercase();
    SECRET_WORDS.iter().any(|word| last.contains(word))
}

#[cfg(test)]
mod tests;
