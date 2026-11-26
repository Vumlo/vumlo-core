//! The `lecture` type (ADR-0020, ADR-0098): a text to read, a block document of the profile
//! `full`. It has no points and nothing is handed in; a student marks it done. Pure code.

use serde_json::{Map, Value};

use crate::{ActivityType, CompletionPolicy, Graded, plain_text};

pub const ID: &str = "lecture";

/// Words per minute of a reading time (ADR-0049).
const WORDS_PER_MINUTE: usize = 200;

/// The minutes a lecture takes to read: its words over 200, rounded up, at least one. `None` when
/// the body is not a document.
pub fn read_minutes(body: &Map<String, Value>) -> Option<u32> {
    let document: api_types::Document = serde_json::from_value(Value::Object(body.clone())).ok()?;
    let words = plain_text(&document).split_whitespace().count();
    u32::try_from(words.div_ceil(WORDS_PER_MINUTE).max(1)).ok()
}

pub struct Lecture;

impl ActivityType for Lecture {
    fn id(&self) -> &'static str {
        ID
    }

    fn completion(&self) -> CompletionPolicy {
        CompletionPolicy::Marked
    }

    fn graded(&self) -> Option<&dyn Graded> {
        None
    }

    fn reading_minutes(&self, body: &Map<String, Value>) -> Option<u32> {
        read_minutes(body)
    }
}

#[cfg(test)]
mod tests;
