//! The assignment types whose submissions are not one block document: `quiz` (ADR-0050),
//! `form` (ADR-0051) and `file` (ADR-0069). Pure code: no database, no HTTP.

pub mod file;
pub mod form;
pub mod no_submission;
pub mod quiz;
mod registry;
pub mod text;

pub use registry::builtin_types;

use activity::{ConfigError, DocumentError, DocumentErrors, MAX_ERRORS};
use serde_json::{Map, Value};
use sqlx::types::Decimal;

use super::model::ModelError;

/// What a type's own functions refuse, as the contract's error. Only `ModelError::Config` comes
/// from a type; its text is the message, and any other variant maps by its text too.
pub(super) fn config_error(error: ModelError) -> ConfigError {
    ConfigError::new(error.to_string())
}

/// Ids of questions, fields and options: 1 to 64 characters of letters, digits, `_` and `-`.
pub(super) fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// A decimal an answer or a rule carries: an optional minus, digits, an optional fraction after `.`
/// or `,`, at most 40 characters. No exponent, no spaces.
pub(super) fn parse_number(raw: &str) -> Option<Decimal> {
    let text = raw.trim();
    if text.is_empty() || text.len() > 40 {
        return None;
    }
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let (whole, fraction) = unsigned.split_once(['.', ',']).unwrap_or((unsigned, ""));
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    let has_separator = unsigned.contains(['.', ',']);
    let shape_ok = !whole.is_empty()
        && digits(whole)
        && digits(fraction)
        && has_separator == !fraction.is_empty();
    if !shape_ok {
        return None;
    }
    text.replace(',', ".").parse().ok()
}

/// The first problem of a nested document as a config error naming where it is.
pub(super) fn document_problem(path: &str, errors: &DocumentErrors) -> ModelError {
    let detail = errors.iter().next().map_or_else(
        || "is not a valid document".to_owned(),
        |error| {
            if error.path.is_empty() {
                format!("is not a valid document: {}", error.message)
            } else {
                format!("is not a valid document: {} {}", error.path, error.message)
            }
        },
    );
    ModelError::Config(format!("`{path}` {detail}"))
}

/// A serde failure as a config error. The message may quote a short piece of what was sent; it is
/// cut so that it cannot repeat a large value.
pub(super) fn shape_problem(what: &str, error: &serde_json::Error) -> ModelError {
    let text = error.to_string();
    let text = text.split(" at line ").next().unwrap_or(&text);
    let cut: String = text.chars().take(160).collect();
    ModelError::Config(format!("the {what} config has the wrong shape: {cut}"))
}

/// Collects answer problems, at most [`MAX_ERRORS`].
#[derive(Default)]
pub(super) struct Problems(Vec<DocumentError>);

impl Problems {
    pub fn add(&mut self, path: impl Into<String>, message: &'static str) {
        if self.0.len() < MAX_ERRORS {
            self.0.push(DocumentError {
                path: path.into(),
                message,
            });
        }
    }

    pub fn into_result<T>(self, ok: impl FnOnce() -> T) -> Result<T, DocumentErrors> {
        if self.0.is_empty() {
            Ok(ok())
        } else {
            Err(DocumentErrors::of(self.0))
        }
    }
}

/// `answers.<id>`, with an id cut to 64 characters so a hostile key cannot make a long path.
pub(super) fn answer_path(id: &str) -> String {
    format!("answers.{}", id.chars().take(64).collect::<String>())
}

/// The `{"answers": {...}}` object of a content, or the reason it is not one. A missing `answers`
/// is an empty one.
pub(super) fn answers_of(content: &Value) -> Result<&Map<String, Value>, DocumentErrors> {
    static EMPTY: std::sync::LazyLock<Map<String, Value>> = std::sync::LazyLock::new(Map::new);
    let Some(content) = content.as_object() else {
        return Err(DocumentErrors::one("", "must be an object"));
    };
    if content.keys().any(|key| key != "answers") {
        return Err(DocumentErrors::one("", "has only `answers`"));
    }
    match content.get("answers") {
        None => Ok(&EMPTY),
        Some(Value::Object(answers)) => Ok(answers),
        Some(_) => Err(DocumentErrors::one("answers", "must be an object")),
    }
}

#[cfg(test)]
mod tests;
