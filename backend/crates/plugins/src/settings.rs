//! What a plugin lets an administrator configure (ADR-0072): the `settings` of its manifest, a
//! subset of JSON Schema, and the rules a value must meet. The core trusts none of it: a spec that
//! is not coherent stops the install, and a value is checked before it is stored.
//!
//! ```json
//! "settings": {
//!   "greeting":  {"type": "string", "default": "Hello", "description": "…"},
//!   "api_token": {"type": "string", "secret": true},
//!   "mode":      {"type": "string", "enum": ["fast", "slow"], "default": "fast"},
//!   "max_words": {"type": "number", "minimum": 0, "default": 100, "scope": "course"},
//!   "tags":      {"type": "array", "items": {"type": "string"}}
//! }
//! ```
//!
//! The five kinds are `string`, `number`, `boolean`, an `enum` (a string with `enum`) and a list
//! (`array` of `items` that are strings or numbers). `scope` is `institution` (the default) or
//! `course`; `secret: true` is for tokens and passwords.

use common::text_enum;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_SETTINGS: usize = 40;
const MAX_KEY_CHARS: usize = 40;
const MAX_DESCRIPTION_CHARS: usize = 300;
const MAX_STRING_CHARS: usize = 2000;
const MAX_LIST_ITEMS: usize = 50;
const MAX_ENUM_OPTIONS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingKind {
    #[default]
    String,
    Number,
    Boolean,
    Array,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingScope {
    /// One value for the whole installation.
    #[default]
    Institution,
    /// One value per course, set apart from the others.
    Course,
}
text_enum!(SettingScope { Institution => "institution", Course => "course" });

/// The type of the entries of a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemSpec {
    #[serde(rename = "type")]
    pub kind: ScalarKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScalarKind {
    String,
    Number,
}

/// One setting as the manifest declares it. This is also how an install stores it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingSpec {
    #[serde(rename = "type")]
    pub kind: SettingKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// A token or a password: stored sealed, never shown by the core to anyone but the plugin.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub secret: bool,
    #[serde(default)]
    pub scope: SettingScope,
    /// The values a string may take.
    #[serde(default, rename = "enum", skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<ItemSpec>,
}

pub fn is_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && key.len() <= MAX_KEY_CHARS
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn plain_text(text: &str, max: usize) -> bool {
    text.chars().count() <= max && !text.chars().any(char::is_control)
}

impl SettingSpec {
    /// Whether the spec says one coherent thing. `key` is only for the message.
    pub fn validate(&self, key: &str) -> Result<(), String> {
        let bad = |why: &str| Err(format!("setting `{key}`: {why}"));
        if !plain_text(&self.description, MAX_DESCRIPTION_CHARS) {
            return bad("the description must be plain text of at most 300 characters");
        }
        let numeric = self.kind == SettingKind::Number;
        if (self.minimum.is_some() || self.maximum.is_some()) && !numeric {
            return bad("`minimum` and `maximum` are for numbers");
        }
        if self
            .minimum
            .zip(self.maximum)
            .is_some_and(|(lo, hi)| lo > hi)
        {
            return bad("`minimum` is above `maximum`");
        }
        if self.minimum.is_some_and(|n| !n.is_finite())
            || self.maximum.is_some_and(|n| !n.is_finite())
        {
            return bad("`minimum` and `maximum` must be finite");
        }
        match (&self.options, self.kind) {
            (Some(options), SettingKind::String) => {
                let mut seen = Vec::new();
                for option in options {
                    if option.is_empty() || !plain_text(option, 100) || seen.contains(&option) {
                        return bad("`enum` lists distinct plain strings of 1 to 100 characters");
                    }
                    seen.push(option);
                }
                if options.is_empty() || options.len() > MAX_ENUM_OPTIONS {
                    return bad("`enum` lists 1 to 50 values");
                }
            }
            (Some(_), _) => return bad("`enum` is for strings"),
            (None, _) => {}
        }
        if self.items.is_some() != (self.kind == SettingKind::Array) {
            return bad("`items` goes with the type `array`, and only with it");
        }
        if self.secret && (self.kind != SettingKind::String || self.options.is_some()) {
            return bad("only a string without `enum` can be a secret");
        }
        if self.secret && self.default.is_some() {
            return bad("a secret has no default");
        }
        if let Some(default) = &self.default {
            self.check(default)
                .map_err(|why| format!("setting `{key}`: its default {why}"))?;
        }
        Ok(())
    }

    /// The value, if it meets the spec: `Err` says why, in words that complete "the value …".
    pub fn check(&self, value: &Value) -> Result<(), String> {
        match (self.kind, value) {
            (SettingKind::String, Value::String(text)) => {
                if !plain_text(text, MAX_STRING_CHARS) {
                    return Err(format!(
                        "must be plain text of at most {MAX_STRING_CHARS} characters"
                    ));
                }
                if self.secret && text.is_empty() {
                    return Err("must not be empty (unset it instead)".to_owned());
                }
                match &self.options {
                    Some(options) if !options.contains(text) => {
                        Err(format!("must be one of: {}", options.join(", ")))
                    }
                    _ => Ok(()),
                }
            }
            (SettingKind::Number, Value::Number(number)) => {
                // serde_json numbers are finite; the bounds may still exclude it.
                let n = number.as_f64().unwrap_or(f64::NAN);
                match (self.minimum, self.maximum) {
                    (Some(lo), _) if n < lo => Err(format!("must be at least {lo}")),
                    (_, Some(hi)) if n > hi => Err(format!("must be at most {hi}")),
                    _ => Ok(()),
                }
            }
            (SettingKind::Boolean, Value::Bool(_)) => Ok(()),
            (SettingKind::Array, Value::Array(entries)) => {
                if entries.len() > MAX_LIST_ITEMS {
                    return Err(format!("must have at most {MAX_LIST_ITEMS} entries"));
                }
                let entry = self.items.as_ref().map_or(ScalarKind::String, |i| i.kind);
                for entry_value in entries {
                    match (entry, entry_value) {
                        (ScalarKind::String, Value::String(text))
                            if plain_text(text, MAX_STRING_CHARS) => {}
                        (ScalarKind::Number, Value::Number(_)) => {}
                        _ => {
                            return Err(format!(
                                "must be a list of {}s",
                                match entry {
                                    ScalarKind::String => "string",
                                    ScalarKind::Number => "number",
                                }
                            ));
                        }
                    }
                }
                Ok(())
            }
            _ => Err(format!("must be a {}", self.kind_name())),
        }
    }

    fn kind_name(&self) -> &'static str {
        match self.kind {
            SettingKind::String if self.options.is_some() => "string from the `enum`",
            SettingKind::String => "string",
            SettingKind::Number => "number",
            SettingKind::Boolean => "boolean",
            SettingKind::Array => "list",
        }
    }

    /// A value typed on a command line: a string stays text, anything else is JSON (`3`, `true`,
    /// `["a","b"]`). Not yet checked against the spec.
    pub fn parse_text(&self, text: &str) -> Result<Value, String> {
        if self.kind == SettingKind::String {
            return Ok(Value::String(text.to_owned()));
        }
        serde_json::from_str(text.trim()).map_err(|_| format!("must be a {}", self.kind_name()))
    }
}

#[cfg(test)]
mod tests;
