//! The `form` type (ADR-0051): the config, the answers, the claims on the course and the facts
//! added to the config when it is read.

use std::collections::{BTreeSet, HashMap, HashSet};

use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sqlx::types::Decimal;
use uuid::Uuid;

use super::{
    Claims, Problems, Stage, ValidContent, answer_path, answers_of, parse_number, shape_problem,
    valid_id,
};
use crate::content::document::{DocumentErrors, valid_href};
use crate::content::model::ModelError;

const MAX_FIELDS: usize = 50;
const MAX_OPTIONS: usize = 20;
const MAX_NOTE_CHARS: usize = 1000;
const MAX_LABEL_CHARS: usize = 200;
const MAX_HELP_CHARS: usize = 500;
const MAX_OPTION_CHARS: usize = 200;
const MAX_PEOPLE: i64 = 10;
const MAX_HREF_CHARS: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FieldType {
    Short,
    Long,
    Choice,
    People,
    Link,
    Number,
    Date,
}

impl FieldType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Short => "short",
            Self::Long => "long",
            Self::Choice => "choice",
            Self::People => "people",
            Self::Link => "link",
            Self::Number => "number",
            Self::Date => "date",
        }
    }

    /// The rules a field of this type may carry.
    fn rules(self) -> &'static [&'static str] {
        match self {
            Self::Short | Self::Long => &["max_length"],
            Self::Choice => &["exclusive", "multiple"],
            Self::People => &["min", "max"],
            Self::Number => &["min", "max"],
            Self::Link | Self::Date => &[],
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    v: u8,
    #[serde(default)]
    note: String,
    fields: Vec<RawField>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawField {
    id: String,
    #[serde(rename = "type")]
    kind: FieldType,
    label: String,
    #[serde(default)]
    help: String,
    #[serde(default)]
    required: bool,
    options: Option<Vec<RawOption>>,
    #[serde(default)]
    rules: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOption {
    id: String,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Field {
    id: String,
    kind: FieldType,
    required: bool,
    max_length: usize,
    options: Vec<String>,
    exclusive: bool,
    multiple: bool,
    min: Option<Decimal>,
    max: Option<Decimal>,
}

fn rule_int(
    rules: &Map<String, Value>,
    key: &str,
    range: std::ops::RangeInclusive<i64>,
    path: &str,
) -> Result<Option<i64>, ModelError> {
    let Some(value) = rules.get(key) else {
        return Ok(None);
    };
    value
        .as_i64()
        .filter(|n| range.contains(n))
        .map(Some)
        .ok_or_else(|| {
            ModelError::Config(format!(
                "`{path}.rules.{key}` must be a whole number from {} to {}",
                range.start(),
                range.end()
            ))
        })
}

fn rule_bool(rules: &Map<String, Value>, key: &str, path: &str) -> Result<bool, ModelError> {
    match rules.get(key) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(ModelError::Config(format!(
            "`{path}.rules.{key}` must be true or false"
        ))),
    }
}

fn rule_decimal(
    rules: &Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<Option<Decimal>, ModelError> {
    match rules.get(key) {
        None => Ok(None),
        Some(Value::String(text)) => parse_number(text)
            .map(Some)
            .ok_or_else(|| ModelError::Config(format!("`{path}.rules.{key}` must be a number"))),
        Some(_) => Err(ModelError::Config(format!(
            "`{path}.rules.{key}` must be a number in quotes"
        ))),
    }
}

fn parse(config: &Map<String, Value>) -> Result<Vec<Field>, ModelError> {
    let bad = |message: String| ModelError::Config(message);
    let raw: RawConfig = serde_json::from_value(Value::Object(config.clone()))
        .map_err(|error| shape_problem("form", &error))?;
    if raw.v != 1 {
        return Err(bad("`v` must be 1".to_owned()));
    }
    if raw.note.chars().count() > MAX_NOTE_CHARS {
        return Err(bad(format!(
            "`note` is longer than {MAX_NOTE_CHARS} characters"
        )));
    }
    if !(1..=MAX_FIELDS).contains(&raw.fields.len()) {
        return Err(bad(format!("`fields` must have 1 to {MAX_FIELDS} fields")));
    }
    let mut seen = BTreeSet::new();
    let mut fields = Vec::with_capacity(raw.fields.len());
    for (index, f) in raw.fields.into_iter().enumerate() {
        let path = format!("fields[{index}]");
        if !valid_id(&f.id) || !seen.insert(f.id.clone()) {
            return Err(bad(format!(
                "`{path}.id` must be 1 to 64 letters, digits, `_` or `-`, and unique"
            )));
        }
        let label = f.label.trim();
        if label.is_empty()
            || label.chars().count() > MAX_LABEL_CHARS
            || label.chars().any(char::is_control)
        {
            return Err(bad(format!(
                "`{path}.label` must be 1 to {MAX_LABEL_CHARS} characters"
            )));
        }
        if f.help.chars().count() > MAX_HELP_CHARS {
            return Err(bad(format!(
                "`{path}.help` is longer than {MAX_HELP_CHARS} characters"
            )));
        }
        if let Some(rule) = f
            .rules
            .keys()
            .find(|rule| !f.kind.rules().contains(&rule.as_str()))
        {
            return Err(bad(format!(
                "`{path}.rules.{rule}` does not belong to a {} field",
                f.kind.as_str()
            )));
        }
        if f.options.is_some() && f.kind != FieldType::Choice {
            return Err(bad(format!(
                "`{path}.options` does not belong to a {} field",
                f.kind.as_str()
            )));
        }
        let mut field = Field {
            id: f.id,
            kind: f.kind,
            required: f.required,
            max_length: 0,
            options: Vec::new(),
            exclusive: false,
            multiple: false,
            min: None,
            max: None,
        };
        match f.kind {
            FieldType::Short | FieldType::Long => {
                let (default, limit) = if f.kind == FieldType::Short {
                    (200, 500)
                } else {
                    (5000, 10_000)
                };
                let given = rule_int(&f.rules, "max_length", 1..=limit, &path)?;
                field.max_length = usize::try_from(given.unwrap_or(default)).unwrap_or(0);
            }
            FieldType::Choice => {
                let options = f
                    .options
                    .ok_or_else(|| bad(format!("`{path}.options` is required")))?;
                if !(2..=MAX_OPTIONS).contains(&options.len()) {
                    return Err(bad(format!(
                        "`{path}.options` must have 2 to {MAX_OPTIONS} options"
                    )));
                }
                let mut ids = BTreeSet::new();
                for (n, option) in options.iter().enumerate() {
                    let text = option.text.trim();
                    if !valid_id(&option.id)
                        || !ids.insert(option.id.clone())
                        || text.is_empty()
                        || text.chars().count() > MAX_OPTION_CHARS
                    {
                        return Err(bad(format!(
                            "`{path}.options[{n}]` needs a unique id and a text of 1 to {MAX_OPTION_CHARS} characters"
                        )));
                    }
                }
                field.options = options.into_iter().map(|option| option.id).collect();
                field.exclusive = rule_bool(&f.rules, "exclusive", &path)?;
                field.multiple = rule_bool(&f.rules, "multiple", &path)?;
            }
            FieldType::People => {
                let min = rule_int(&f.rules, "min", 0..=MAX_PEOPLE, &path)?.unwrap_or(0);
                let max = rule_int(&f.rules, "max", 0..=MAX_PEOPLE, &path)?.unwrap_or(5);
                if min > max {
                    return Err(bad(format!("`{path}.rules.min` is above `max`")));
                }
                field.min = Some(Decimal::from(min));
                field.max = Some(Decimal::from(max));
            }
            FieldType::Number => {
                field.min = rule_decimal(&f.rules, "min", &path)?;
                field.max = rule_decimal(&f.rules, "max", &path)?;
                if field.min.zip(field.max).is_some_and(|(min, max)| min > max) {
                    return Err(bad(format!("`{path}.rules.min` is above `max`")));
                }
            }
            FieldType::Link | FieldType::Date => {}
        }
        fields.push(field);
    }
    Ok(fields)
}

/// Checks a form config (ADR-0051 points 1 and 2).
pub fn validate_config(config: &Map<String, Value>) -> Result<(), ModelError> {
    parse(config).map(|_| ())
}

/// Drops the facts the server adds when it reads a config, so that a client that sends back what
/// it read does not store them (ADR-0051 point 5).
pub fn strip_computed(config: &mut Map<String, Value>) {
    let Some(Value::Array(fields)) = config.get_mut("fields") else {
        return;
    };
    for field in fields.iter_mut().filter_map(Value::as_object_mut) {
        field.remove("has_answers");
        if let Some(Value::Array(options)) = field.get_mut("options") {
            for option in options.iter_mut().filter_map(Value::as_object_mut) {
                option.remove("taken");
                option.remove("taken_by");
            }
        }
    }
}

/// The part of a config that cannot change once a form has been submitted (ADR-0051 point 6).
pub fn structure(config: &Map<String, Value>) -> Option<Value> {
    parse(config).ok()?;
    let fields = config.get("fields")?.as_array()?;
    Some(Value::Array(
        fields
            .iter()
            .filter_map(Value::as_object)
            .map(|field| {
                let options: Vec<&Value> = field
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|options| {
                        options
                            .iter()
                            .filter_map(|option| option.get("id"))
                            .collect()
                    })
                    .unwrap_or_default();
                let mut options: Vec<String> = options
                    .into_iter()
                    .filter_map(|id| id.as_str().map(str::to_owned))
                    .collect();
                options.sort();
                json!({
                    "id": field.get("id"),
                    "type": field.get("type"),
                    "required": field.get("required").and_then(Value::as_bool).unwrap_or(false),
                    "rules": field.get("rules").cloned().unwrap_or_else(|| json!({})),
                    "options": options,
                })
            })
            .collect(),
    ))
}

/// A value that counts as no answer.
fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    }
}

fn string_list(value: &Value) -> Option<Vec<&str>> {
    value
        .as_array()?
        .iter()
        .map(Value::as_str)
        .collect::<Option<Vec<_>>>()
}

/// Checks the answers of a draft or an attempt (ADR-0051 point 3).
pub fn validate_submission(
    config: &Map<String, Value>,
    content: &Value,
    stage: Stage,
) -> Result<ValidContent, DocumentErrors> {
    let fields = parse(config).map_err(|_| DocumentErrors::one("", "the form is misconfigured"))?;
    let answers = answers_of(content)?;
    let mut problems = Problems::default();
    let mut kept = Map::new();
    for (id, value) in answers {
        let path = answer_path(id);
        let Some(field) = fields.iter().find(|f| &f.id == id) else {
            problems.add(path, "is not a field of this form");
            continue;
        };
        if is_empty(value) {
            continue;
        }
        match check_value(field, value, stage) {
            Ok(stored) => {
                kept.insert(id.clone(), stored);
            }
            Err(message) => problems.add(path, message),
        }
    }
    if stage == Stage::Submit {
        for field in fields.iter().filter(|f| f.required) {
            if answers.get(&field.id).is_none_or(is_empty) {
                problems.add(answer_path(&field.id), "is required");
            }
        }
    }
    problems.into_result(|| {
        let blank = kept.is_empty();
        let mut body = Map::new();
        body.insert("answers".to_owned(), Value::Object(kept));
        ValidContent::new(body, blank)
    })
}

/// One non-empty answer against its field: the value to store, or what is wrong.
fn check_value(field: &Field, value: &Value, stage: Stage) -> Result<Value, &'static str> {
    match field.kind {
        FieldType::Short | FieldType::Long => {
            let text = value.as_str().ok_or("must be text")?.trim();
            if text.chars().count() > field.max_length {
                return Err("is longer than the field allows");
            }
            let allowed = |c: char| {
                !c.is_control()
                    || (field.kind == FieldType::Long && matches!(c, '\n' | '\r' | '\t'))
            };
            if !text.chars().all(allowed) {
                return Err("has a control character");
            }
            Ok(Value::String(text.to_owned()))
        }
        FieldType::Choice => {
            let picked = string_list(value).ok_or("must be a list of option ids")?;
            let distinct: BTreeSet<&&str> = picked.iter().collect();
            if distinct.len() != picked.len()
                || picked
                    .iter()
                    .any(|id| !field.options.iter().any(|o| o == id))
                || (!field.multiple && picked.len() > 1)
            {
                return Err(
                    "must name options of the field, once each, and one unless it takes several",
                );
            }
            Ok(value.clone())
        }
        FieldType::People => {
            let picked = string_list(value).ok_or("must be a list of user ids")?;
            let ids: BTreeSet<Uuid> = picked.iter().filter_map(|id| id.parse().ok()).collect();
            if ids.len() != picked.len() {
                return Err("must be a list of different user ids");
            }
            let count = Decimal::from(picked.len());
            if field.max.is_some_and(|max| count > max) {
                return Err("names more people than the field allows");
            }
            if stage == Stage::Submit && field.min.is_some_and(|min| count < min) {
                return Err("names fewer people than the field needs");
            }
            Ok(value.clone())
        }
        FieldType::Link => {
            let text = value.as_str().ok_or("must be text")?.trim();
            if text.chars().count() > MAX_HREF_CHARS || !valid_href(text) {
                return Err("must be an http or https address of at most 2048 characters");
            }
            Ok(Value::String(text.to_owned()))
        }
        FieldType::Number => {
            let text = value.as_str().ok_or("must be a number")?.trim();
            let number = parse_number(text).ok_or("must be a number")?;
            if field.min.is_some_and(|min| number < min)
                || field.max.is_some_and(|max| number > max)
            {
                return Err("is outside the range the field allows");
            }
            Ok(Value::String(text.to_owned()))
        }
        FieldType::Date => {
            let text = value.as_str().ok_or("must be a date")?.trim();
            if text.len() != 10 || NaiveDate::parse_from_str(text, "%Y-%m-%d").is_err() {
                return Err("must be a date like 2026-10-23");
            }
            Ok(Value::String(text.to_owned()))
        }
    }
}

/// What the content of an attempt asks of the rest of the course (ADR-0051 points 3 and 4).
pub fn claims(config: &Map<String, Value>, content: &Map<String, Value>) -> Claims {
    let Ok(fields) = parse(config) else {
        return Claims::default();
    };
    let Some(answers) = content.get("answers").and_then(Value::as_object) else {
        return Claims::default();
    };
    let mut claims = Claims::default();
    for field in &fields {
        let Some(list) = answers.get(&field.id).and_then(string_list) else {
            continue;
        };
        match field.kind {
            FieldType::People => claims
                .people
                .extend(list.iter().filter_map(|id| id.parse::<Uuid>().ok())),
            FieldType::Choice if field.exclusive => claims.exclusive.extend(
                list.iter()
                    .map(|option| (field.id.clone(), (*option).to_owned())),
            ),
            _ => {}
        }
    }
    claims
}

/// The holder of an exclusive option: the student and the name to show staff.
pub type Holders = HashMap<(String, String), (Uuid, String)>;

/// Adds to a config the facts of ADR-0051 point 5: whether a field has answers (staff), and for
/// each option of an exclusive field whether another student holds it and, for staff, who.
pub fn decorate(
    config: &mut Map<String, Value>,
    answered: &HashSet<String>,
    holders: &Holders,
    staff: bool,
    viewer: Uuid,
) {
    let Some(Value::Array(fields)) = config.get_mut("fields") else {
        return;
    };
    for field in fields.iter_mut().filter_map(Value::as_object_mut) {
        let id = field
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if staff {
            field.insert(
                "has_answers".to_owned(),
                Value::Bool(answered.contains(&id)),
            );
        }
        let exclusive = field
            .get("rules")
            .and_then(|rules| rules.get("exclusive"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !exclusive {
            continue;
        }
        let Some(Value::Array(options)) = field.get_mut("options") else {
            continue;
        };
        for option in options.iter_mut().filter_map(Value::as_object_mut) {
            let option_id = option
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let holder = holders.get(&(id.clone(), option_id));
            option.insert(
                "taken".to_owned(),
                Value::Bool(holder.is_some_and(|(student, _)| *student != viewer)),
            );
            if staff {
                option.insert(
                    "taken_by".to_owned(),
                    holder.map_or(Value::Null, |(_, name)| Value::String(name.clone())),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests;
