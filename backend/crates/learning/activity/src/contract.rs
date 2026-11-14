use std::collections::BTreeMap;

use serde_json::{Map, Value};
use sqlx::types::Decimal;

use crate::document::ValidDocument;

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
    pub fn new(body: Map<String, Value>, blank: bool) -> Self {
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
    /// The files of a `file` assignment's submission; the submitter must be allowed to use them.
    pub files: Vec<uuid::Uuid>,
}

impl Claims {
    pub fn is_empty(&self) -> bool {
        self.people.is_empty() && self.exclusive.is_empty() && self.files.is_empty()
    }
}
