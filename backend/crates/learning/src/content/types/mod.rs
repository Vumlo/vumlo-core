//! The assignment types whose submissions are not one block document: `quiz` (ADR-0050) and
//! `form` (ADR-0051). Pure code: no database, no HTTP.

pub mod form;
pub mod quiz;

use std::collections::BTreeMap;

use serde_json::{Map, Value};
use sqlx::types::Decimal;

use super::document::{DocumentError, DocumentErrors, MAX_ERRORS, ValidDocument};
use super::model::ModelError;

/// Whether a content is being saved as a draft or submitted. A draft may be unfinished: the
/// required fields of a form are judged only at the submit (ADR-0051 point 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Draft,
    Submit,
}

/// The content of a submission after the type checked it, in the form that is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidContent {
    body: Map<String, Value>,
    blank: bool,
}

impl ValidContent {
    pub(super) fn new(body: Map<String, Value>, blank: bool) -> Self {
        Self { body, blank }
    }

    pub fn as_body(&self) -> &Map<String, Value> {
        &self.body
    }

    /// True when nothing was answered: an empty draft may be saved but never submitted
    /// (ADR-0035 point 3).
    pub fn is_blank(&self) -> bool {
        self.blank
    }
}

impl From<ValidDocument> for ValidContent {
    fn from(document: ValidDocument) -> Self {
        Self {
            blank: document.is_blank(),
            body: document.as_body().clone(),
        }
    }
}

/// What a type could score by itself when a student submitted (ADR-0050 point 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoGrade {
    /// Question id to score, for the questions that were scored.
    pub scores: BTreeMap<String, Decimal>,
    pub total: Decimal,
    /// Every question was scored: only then is there a draft grade to write (ADR-0050 point 5).
    pub complete: bool,
}

impl AutoGrade {
    /// The stored form: question id to a decimal string with two decimals.
    pub fn scores_json(&self) -> Value {
        Value::Object(
            self.scores
                .iter()
                .map(|(id, score)| (id.clone(), Value::String(format!("{score:.2}"))))
                .collect(),
        )
    }
}

/// What a submission claims from the rest of the course (ADR-0051 points 3 and 4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Claims {
    /// The users named in `people` fields; they must be current students.
    pub people: Vec<uuid::Uuid>,
    /// `(field id, option id)` of the exclusive options chosen.
    pub exclusive: Vec<(String, String)>,
}

impl Claims {
    pub fn is_empty(&self) -> bool {
        self.people.is_empty() && self.exclusive.is_empty()
    }
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
