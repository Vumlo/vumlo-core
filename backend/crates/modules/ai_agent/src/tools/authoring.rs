//! The write tools that create content (ADR-0070): a module, a text item, an assignment, a quiz.
//! Everything is created as a draft, which students never see; publishing stays the teacher's act in
//! the course screens. The model sends plain text and lists; the documents and the quiz config the
//! core validates are built here, so the model never writes their JSON.

use std::collections::BTreeMap;

use access::Actor;
use common::ApiError;
use learning::content::{
    self, Assignment, BuiltinType, ContentTitle, ItemRules, MaxPoints, NewItemInput,
};
use learning::items;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;
use sqlx::types::Decimal;
use uuid::Uuid;

use super::grading::{explain, text_or_number};
use crate::llm::ToolSpec;

/// Questions one change may carry: what a person can read and check on a card.
const MAX_QUESTIONS: usize = 30;
/// Characters of a preview the card shows.
const PREVIEW_CHARS: usize = 6000;
/// The platform's limit of options per choice question.
const MAX_OPTIONS: usize = 10;

pub const NAMES: [&str; 4] = [
    "create_module",
    "create_text_item",
    "create_assignment",
    "create_quiz",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    SingleChoice,
    MultipleChoice,
    Number,
    ShortText,
    LongText,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::SingleChoice => "single_choice",
            Self::MultipleChoice => "multiple_choice",
            Self::Number => "number",
            Self::ShortText => "short_text",
            Self::LongText => "long_text",
        }
    }
}

/// One quiz question as the model writes it. Options are plain texts; `correct` counts them from 0.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Question {
    pub prompt: String,
    #[serde(rename = "type")]
    pub kind: Kind,
    #[serde(deserialize_with = "text_or_number")]
    pub points: Option<String>,
    #[serde(default)]
    pub options: Option<Vec<String>>,
    #[serde(default)]
    pub correct: Option<Vec<usize>>,
    #[serde(default, deserialize_with = "text_or_number")]
    pub answer: Option<String>,
    #[serde(default, deserialize_with = "text_or_number")]
    pub tolerance: Option<String>,
    #[serde(default)]
    pub accepted: Option<Vec<String>>,
}

/// A piece of content to create, as a draft.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tool", deny_unknown_fields)]
pub enum Write {
    #[serde(rename = "create_module")]
    Module { course_id: Uuid, title: String },
    #[serde(rename = "create_text_item")]
    TextItem {
        course_id: Uuid,
        module_id: Uuid,
        title: String,
        text: String,
    },
    #[serde(rename = "create_assignment")]
    Assignment {
        course_id: Uuid,
        module_id: Uuid,
        title: String,
        /// The instructions; paragraphs are separated by a blank line.
        #[serde(default)]
        text: Option<String>,
        #[serde(deserialize_with = "text_or_number")]
        max_points: Option<String>,
    },
    #[serde(rename = "create_quiz")]
    Quiz {
        course_id: Uuid,
        module_id: Uuid,
        title: String,
        #[serde(default)]
        text: Option<String>,
        questions: Vec<Question>,
    },
}

pub fn specs() -> Vec<ToolSpec> {
    let id = |what: &str| json!({"type": "string", "description": what});
    let item = |extra: Value, required: &[&str]| {
        let mut properties = json!({
            "course_id": id("A course id from teaching_overview"),
            "module_id": id("A module id from course_content, in that course"),
            "title": {"type": "string"},
        });
        properties
            .as_object_mut()
            .expect("an object")
            .extend(extra.as_object().expect("an object").clone());
        let mut required = required.to_vec();
        required.extend(["course_id", "module_id", "title"]);
        json!({"type": "object", "properties": properties, "required": required})
    };
    let question = json!({
        "type": "object",
        "properties": {
            "prompt": {"type": "string"},
            "type": {"type": "string", "enum": ["single_choice", "multiple_choice", "number", "short_text", "long_text"]},
            "points": {"type": ["number", "string"], "description": "Above 0, at most two decimals"},
            "options": {"type": "array", "items": {"type": "string"}, "description": "choice questions: 2 to 10 options"},
            "correct": {"type": "array", "items": {"type": "integer"}, "description": "choice questions: the positions of the correct options, counting from 0; exactly one for single_choice"},
            "answer": {"type": ["number", "string"], "description": "number questions: the correct number"},
            "tolerance": {"type": ["number", "string"], "description": "number questions: how far off still counts, default 0"},
            "accepted": {"type": "array", "items": {"type": "string"}, "description": "short_text questions: the answers that count (case ignored)"}
        },
        "required": ["prompt", "type", "points"]
    });
    vec![
        ToolSpec::new(
            "create_module",
            "Prepares creating a new, unpublished module (a section) in a course. The user must \
             confirm before it is created.",
            json!({
                "type": "object",
                "properties": {"course_id": id("A course id from teaching_overview"), "title": {"type": "string"}},
                "required": ["course_id", "title"]
            }),
        ),
        ToolSpec::new(
            "create_text_item",
            "Prepares creating an unpublished text page (a lesson or a note) in a module. Paragraphs \
             are separated by a blank line. The user must confirm before it is created.",
            item(json!({"text": {"type": "string"}}), &["text"]),
        ),
        ToolSpec::new(
            "create_assignment",
            "Prepares creating an unpublished assignment that students answer with text. `text` holds \
             the instructions. The user must confirm before it is created.",
            item(
                json!({"text": {"type": "string"}, "max_points": {"type": ["number", "string"]}}),
                &["max_points"],
            ),
        ),
        ToolSpec::new(
            "create_quiz",
            "Prepares creating an unpublished quiz. Choice, number and short-text questions are \
             scored automatically; long_text ones are graded by the teacher. Be careful that the \
             correct answers are right: the user reviews them before confirming.",
            item(
                json!({"text": {"type": "string", "description": "optional introduction"}, "questions": {"type": "array", "items": question}}),
                &["questions"],
            ),
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
        let write: Self = serde_json::from_value(value)
            .map_err(|e| format!("the arguments do not fit {name}: {e}"))?;
        write.check()?;
        Ok(write)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Module { .. } => NAMES[0],
            Self::TextItem { .. } => NAMES[1],
            Self::Assignment { .. } => NAMES[2],
            Self::Quiz { .. } => NAMES[3],
        }
    }

    /// What can be refused without asking the core: a missing field, an answer key that does not fit.
    fn check(&self) -> Result<(), String> {
        match self {
            Self::Assignment {
                max_points: None, ..
            } => Err("max_points is required".to_owned()),
            Self::Quiz { questions, .. } => quiz(questions).map(|_| ()),
            _ => Ok(()),
        }
    }

    /// What the card says, read from the core as the user, and a preview of the content.
    pub async fn describe(
        &self,
        conn: &mut PgConnection,
        actor: &Actor,
    ) -> Result<BTreeMap<&'static str, String>, String> {
        let (course_id, title) = match self {
            Self::Module { course_id, title }
            | Self::TextItem {
                course_id, title, ..
            }
            | Self::Assignment {
                course_id, title, ..
            }
            | Self::Quiz {
                course_id, title, ..
            } => (*course_id, title),
        };
        let course = teaching::get_course(conn, actor, course_id)
            .await
            .map_err(|e| explain(&ApiError::from(e)))?;
        let mut facts =
            BTreeMap::from([("course", course.title), ("title", title.trim().to_owned())]);
        if let Self::TextItem { module_id, .. }
        | Self::Assignment { module_id, .. }
        | Self::Quiz { module_id, .. } = self
        {
            let tree = content::get_tree(conn, actor, course_id)
                .await
                .map_err(|e| explain(&ApiError::from(e)))?;
            let module = tree
                .iter()
                .map(|m| &m.module)
                .chain(
                    tree.iter()
                        .flat_map(|m| m.submodules.iter().map(|s| &s.module)),
                )
                .find(|m| m.id == *module_id)
                .ok_or("that module is not in that course")?;
            facts.insert("module", module.title.clone());
        }
        match self {
            Self::Module { .. } => {}
            Self::TextItem { text, .. } => {
                facts.insert("preview", cut(text));
            }
            Self::Assignment {
                text, max_points, ..
            } => {
                facts.insert("max_points", max_points.clone().unwrap_or_default());
                facts.insert("preview", cut(text.as_deref().unwrap_or_default()));
            }
            Self::Quiz {
                text, questions, ..
            } => {
                let (_, total) = quiz(questions)?;
                facts.insert("max_points", total.to_string());
                facts.insert("questions", questions.len().to_string());
                let intro = text.as_deref().filter(|t| !t.trim().is_empty());
                let body = questions
                    .iter()
                    .enumerate()
                    .map(|(n, q)| preview(n + 1, q))
                    .collect::<Vec<_>>()
                    .join("\n");
                facts.insert(
                    "preview",
                    cut(&[intro.unwrap_or_default(), &body].join("\n\n")),
                );
            }
        }
        Ok(facts)
    }

    /// Creates the draft. The caller owns the transaction.
    pub async fn run(
        &self,
        conn: &mut PgConnection,
        actor: &Actor,
        rules: ItemRules<'_>,
    ) -> Result<(), ApiError> {
        let name = |title: &str| {
            ContentTitle::parse(title).map_err(|e| ApiError::invalid_field("title", e.to_string()))
        };
        match self {
            Self::Module { course_id, title } => {
                content::create_module(conn, actor, *course_id, name(title)?, None)
                    .await
                    .map_err(ApiError::from)?;
            }
            Self::TextItem {
                module_id,
                title,
                text,
                ..
            } => {
                items::create_item(
                    conn,
                    actor,
                    rules,
                    *module_id,
                    NewItemInput::document(name(title)?, Some(document(text)), None),
                )
                .await
                .map_err(ApiError::from)?;
            }
            Self::Assignment {
                module_id,
                title,
                text,
                max_points,
                ..
            } => {
                let points = points(max_points.as_deref().unwrap_or_default(), "max_points")?;
                let body = text
                    .as_deref()
                    .filter(|t| !t.trim().is_empty())
                    .map(document);
                let assignment = assignment(BuiltinType::Text, Map::new(), points);
                items::create_item(
                    conn,
                    actor,
                    rules,
                    *module_id,
                    NewItemInput::document(name(title)?, body, Some(assignment)),
                )
                .await
                .map_err(ApiError::from)?;
            }
            Self::Quiz {
                module_id,
                title,
                text,
                questions,
                ..
            } => {
                let (config, total) =
                    quiz(questions).map_err(|m| ApiError::invalid_field("questions", m))?;
                let body = text
                    .as_deref()
                    .filter(|t| !t.trim().is_empty())
                    .map(document);
                let assignment = assignment(
                    BuiltinType::Quiz,
                    config,
                    points(&total.to_string(), "questions")?,
                );
                items::create_item(
                    conn,
                    actor,
                    rules,
                    *module_id,
                    NewItemInput::document(name(title)?, body, Some(assignment)),
                )
                .await
                .map_err(ApiError::from)?;
            }
        }
        Ok(())
    }
}

fn assignment(kind: BuiltinType, config: Map<String, Value>, max_points: MaxPoints) -> Assignment {
    Assignment {
        assignment_type: kind,
        config,
        max_points,
        is_bonus: false,
        settings: Default::default(),
        class_on: None,
    }
}

fn points(raw: &str, field: &str) -> Result<MaxPoints, ApiError> {
    MaxPoints::parse(raw.trim()).map_err(|_| {
        ApiError::invalid_field(field, "points must be above 0, with at most two decimals")
    })
}

/// A document of paragraphs: blank lines separate them. Plain text only, so it is valid in every
/// editor profile.
pub fn document(text: &str) -> Value {
    let blocks: Vec<Value> = text
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .enumerate()
        .map(|(i, p)| json!({"id": format!("p{}", i + 1), "type": "paragraph", "content": [{"type": "text", "text": p}]}))
        .collect();
    json!({"v": 1, "blocks": blocks})
}

/// The quiz config the core validates, and the points it adds up to. The answer key is checked
/// here for what a model gets wrong most: an index that is not an option, or the wrong number of
/// correct ones.
fn quiz(questions: &[Question]) -> Result<(Map<String, Value>, Decimal), String> {
    if questions.is_empty() || questions.len() > MAX_QUESTIONS {
        return Err(format!("a quiz needs 1 to {MAX_QUESTIONS} questions"));
    }
    let mut total = Decimal::ZERO;
    let mut out = Vec::with_capacity(questions.len());
    for (n, q) in questions.iter().enumerate() {
        let at = format!("question {}", n + 1);
        if q.prompt.trim().is_empty() {
            return Err(format!("{at}: the prompt is empty"));
        }
        let raw = q
            .points
            .as_deref()
            .ok_or_else(|| format!("{at}: points is required"))?;
        let value = MaxPoints::parse(raw.trim())
            .map_err(|_| format!("{at}: points must be above 0, with at most two decimals"))?;
        total += value.value();
        let mut item = json!({
            "id": format!("q{}", n + 1),
            "type": q.kind.as_str(),
            "points": value.to_string(),
            "prompt": document(&q.prompt),
        });
        let object = item.as_object_mut().expect("an object");
        match q.kind {
            Kind::SingleChoice | Kind::MultipleChoice => {
                let options = q.options.as_deref().unwrap_or_default();
                if !(2..=MAX_OPTIONS).contains(&options.len())
                    || options.iter().any(|o| o.trim().is_empty())
                {
                    return Err(format!(
                        "{at}: a choice question needs 2 to {MAX_OPTIONS} non-empty options"
                    ));
                }
                let correct = q.correct.as_deref().unwrap_or_default();
                let mut positions = correct.to_vec();
                positions.sort_unstable();
                positions.dedup();
                if positions.len() != correct.len()
                    || positions.is_empty()
                    || positions.iter().any(|p| *p >= options.len())
                {
                    return Err(format!(
                        "{at}: `correct` must list distinct positions of options, counting from 0"
                    ));
                }
                if q.kind == Kind::SingleChoice && positions.len() != 1 {
                    return Err(format!(
                        "{at}: a single_choice question has exactly one correct option"
                    ));
                }
                object.insert(
                    "options".to_owned(),
                    options
                        .iter()
                        .enumerate()
                        .map(|(i, o)| json!({"id": option_id(i), "content": document(o)}))
                        .collect(),
                );
                object.insert(
                    "correct".to_owned(),
                    positions.iter().map(|p| json!(option_id(*p))).collect(),
                );
            }
            Kind::Number => {
                let answer = q
                    .answer
                    .as_deref()
                    .ok_or_else(|| format!("{at}: a number question needs `answer`"))?;
                object.insert("answer".to_owned(), json!(answer.trim()));
                if let Some(tolerance) = &q.tolerance {
                    object.insert("tolerance".to_owned(), json!(tolerance.trim()));
                }
            }
            Kind::ShortText => {
                let accepted = q.accepted.as_deref().unwrap_or_default();
                if accepted.is_empty() || accepted.iter().any(|a| a.trim().is_empty()) {
                    return Err(format!(
                        "{at}: a short_text question needs `accepted` answers"
                    ));
                }
                object.insert(
                    "accepted".to_owned(),
                    json!(accepted.iter().map(|a| a.trim()).collect::<Vec<_>>()),
                );
            }
            Kind::LongText => {}
        }
        out.push(item);
    }
    let config = json!({"v": 1, "questions": out, "show_correct": false});
    Ok((config.as_object().expect("an object").clone(), total))
}

/// `a`, `b`, `c`, ... for the options of a question.
fn option_id(index: usize) -> String {
    char::from(b'a' + u8::try_from(index).unwrap_or(0)).to_string()
}

/// One question as a person reads it on the card, with the correct answers marked.
fn preview(n: usize, q: &Question) -> String {
    let points = q.points.as_deref().unwrap_or("?");
    let mut text = format!("{n}. {} ({points})", q.prompt.trim());
    match q.kind {
        Kind::SingleChoice | Kind::MultipleChoice => {
            let correct = q.correct.as_deref().unwrap_or_default();
            for (i, option) in q.options.as_deref().unwrap_or_default().iter().enumerate() {
                let mark = if correct.contains(&i) { " ✓" } else { "" };
                text.push_str(&format!("\n   {}) {}{mark}", option_id(i), option.trim()));
            }
        }
        Kind::Number => {
            let tolerance = q
                .tolerance
                .as_deref()
                .map(|t| format!(" ± {t}"))
                .unwrap_or_default();
            text.push_str(&format!(
                "\n   = {}{tolerance}",
                q.answer.as_deref().unwrap_or("?")
            ));
        }
        Kind::ShortText => text.push_str(&format!(
            "\n   = {}",
            q.accepted.as_deref().unwrap_or_default().join(" | ")
        )),
        Kind::LongText => {}
    }
    text
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
