//! The write tools of a student (ADR-0070): save a draft of their own answer, and submit the saved
//! draft. Both are proposals the student confirms, run by the core's own `assessment` service as
//! them, so the deadline, the attempts and the events are the core's.
//!
//! "Coach, never author" is enforced here and not only asked of the model: the text of a draft must
//! be words the student wrote in this chat ([`quoted`]). Text the model composed, or that only
//! appears in the model's own answers, never becomes a proposal.

use std::collections::BTreeMap;

use access::Actor;
use chrono::Utc;
use common::{ApiError, PageParams};
use content::{BuiltinType, ItemRules};
use learning::{items, submissions};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use super::grading::explain;
use crate::llm::ToolSpec;

/// Characters of a preview the card shows.
const PREVIEW_CHARS: usize = 6000;

pub const NAMES: [&str; 2] = ["save_draft", "submit_work"];

/// A change to the student's own work.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tool", deny_unknown_fields)]
pub enum Write {
    /// Replaces the student's draft with this text, which they wrote.
    #[serde(rename = "save_draft")]
    SaveDraft { assignment_id: Uuid, text: String },
    /// Turns the saved draft into an attempt. Final.
    #[serde(rename = "submit_work")]
    SubmitWork { assignment_id: Uuid },
}

pub fn specs() -> Vec<ToolSpec> {
    let assignment = json!({"type": "string", "description": "An assignment id from my_work"});
    vec![
        ToolSpec::new(
            "save_draft",
            "Prepares saving a draft of the student's answer to a text assignment. `text` must be \
             words the student wrote in this chat, copied exactly: you never write, complete, \
             improve or rephrase it. It replaces any earlier draft. The student must confirm.",
            json!({
                "type": "object",
                "properties": {"assignment_id": assignment, "text": {"type": "string", "description": "The student's own words, as they wrote them"}},
                "required": ["assignment_id", "text"]
            }),
        ),
        ToolSpec::new(
            "submit_work",
            "Prepares submitting the student's saved draft of a text assignment. It cannot be undone \
             and counts as an attempt. Only after the student has asked to submit. The student must \
             confirm.",
            json!({
                "type": "object",
                "properties": {"assignment_id": assignment},
                "required": ["assignment_id"]
            }),
        ),
    ]
}

impl Write {
    pub fn parse(name: &str, arguments: &str) -> Result<Self, String> {
        let bad = || "the arguments do not fit this tool".to_owned();
        let raw = if arguments.trim().is_empty() {
            "{}"
        } else {
            arguments
        };
        let mut value: Value = serde_json::from_str(raw).map_err(|_| bad())?;
        value
            .as_object_mut()
            .ok_or_else(bad)?
            .insert("tool".to_owned(), Value::String(name.to_owned()));
        serde_json::from_value(value).map_err(|e| format!("the arguments do not fit {name}: {e}"))
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::SaveDraft { .. } => NAMES[0],
            Self::SubmitWork { .. } => NAMES[1],
        }
    }

    /// The authorship rule: a draft is the student's own words. `own_words` are the messages the
    /// student wrote in this chat.
    pub fn check_words(&self, own_words: &[String]) -> Result<(), String> {
        match self {
            Self::SaveDraft { text, .. } if !quoted(text, own_words) => Err(
                "the text is not words the student wrote in this chat; ask them to write or paste \
                 their answer here, and never write it for them"
                    .to_owned(),
            ),
            _ => Ok(()),
        }
    }

    /// What the card shows, read from the core as the student: the assignment and the text that
    /// will be saved or submitted.
    pub async fn describe(
        &self,
        conn: &mut PgConnection,
        actor: &Actor,
    ) -> Result<BTreeMap<&'static str, String>, String> {
        let assignment_id = match self {
            Self::SaveDraft { assignment_id, .. } | Self::SubmitWork { assignment_id } => {
                *assignment_id
            }
        };
        let item = items::get(conn, actor, assignment_id)
            .await
            .map_err(|e| explain(&ApiError::from(e)))?;
        let is_text = item
            .assignment
            .as_ref()
            .is_some_and(|a| a.assignment_type == BuiltinType::Text);
        if !is_text {
            return Err(
                "only text assignments can be answered here; the student does this one on its page"
                    .to_owned(),
            );
        }
        let mut facts = BTreeMap::from([("assignment", item.title.clone())]);
        let text = match self {
            Self::SaveDraft { text, .. } => text.clone(),
            Self::SubmitWork { .. } => {
                let mine = assessment::my_submission(
                    conn,
                    actor,
                    assignment_id,
                    PageParams {
                        after: None,
                        limit: 1,
                    },
                )
                .await
                .map_err(|e| explain(&ApiError::from(e)))?;
                let draft = mine
                    .draft
                    .ok_or("there is no saved draft to submit; a draft is saved first, and the student confirms that")?;
                text_of(&draft.content)
            }
        };
        facts.insert("preview", cut(&text));
        Ok(facts)
    }

    /// Makes the change as the student. The caller owns the transaction.
    pub async fn run(
        &self,
        conn: &mut PgConnection,
        actor: &Actor,
        rules: ItemRules<'_>,
    ) -> Result<(), ApiError> {
        let now = Utc::now();
        match self {
            Self::SaveDraft {
                assignment_id,
                text,
            } => {
                submissions::save_draft(
                    conn,
                    actor,
                    *assignment_id,
                    &student_document(text),
                    rules.profiles,
                    now,
                )
                .await
                .map_err(ApiError::from)?;
            }
            Self::SubmitWork { assignment_id } => {
                submissions::submit(conn, actor, *assignment_id, rules.profiles, now)
                    .await
                    .map_err(ApiError::from)?;
            }
        }
        Ok(())
    }
}

/// Collapses every run of whitespace to one space, so a line break the chat added is no difference.
fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `text` is, apart from whitespace, part of one message the student wrote.
pub fn quoted(text: &str, own_words: &[String]) -> bool {
    let wanted = squash(text);
    !wanted.is_empty() && own_words.iter().any(|m| squash(m).contains(&wanted))
}

/// The content of a student's answer to a text assignment: one paragraph per line.
pub fn student_document(text: &str) -> Value {
    let blocks: Vec<Value> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .enumerate()
        .map(|(i, line)| json!({"id": format!("p{}", i + 1), "type": "paragraph", "content": [{"type": "text", "text": line}]}))
        .collect();
    json!({"v": 1, "attachments": [], "blocks": blocks})
}

/// The text of a stored document, a line per block: what a card or a model reads. Formatting and
/// non-text blocks are left out.
pub fn text_of(content: &Map<String, Value>) -> String {
    content
        .get("blocks")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .map(block_text)
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn block_text(value: &Value) -> String {
    match value {
        Value::Object(object) => match object.get("text") {
            Some(Value::String(text)) => text.clone(),
            _ => object.get("content").map(block_text).unwrap_or_default(),
        },
        Value::Array(items) => items.iter().map(block_text).collect::<Vec<_>>().join(""),
        _ => String::new(),
    }
}

fn cut(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= PREVIEW_CHARS {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(PREVIEW_CHARS).collect();
    short.push('…');
    short
}

#[cfg(test)]
mod tests;
