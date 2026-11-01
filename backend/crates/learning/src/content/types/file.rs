//! The `file` type (ADR-0069): the config, the content `{"files": [id], "comment"?: document}` and
//! the claims on the files. Pure code: what needs the file rows (names, owners) is checked by the
//! assessment service, which calls [`check_names`] once it has the names.

use std::collections::{BTreeMap, BTreeSet};

use api_types::ProfileName;
use serde_json::{Map, Value};
use uuid::Uuid;

use super::{Claims, Problems, Stage, ValidContent};
use crate::content::document::{
    DocumentErrors, EffectiveProfiles, FileRef, file_refs_under, validate_document,
};
use crate::content::model::ModelError;

const DEFAULT_MAX_FILES: usize = 5;
const MAX_FILES_LIMIT: usize = 20;
const MAX_ACCEPT: usize = 30;
const EXTENSION_CHARS: std::ops::RangeInclusive<usize> = 1..=16;

struct Settings {
    max_files: usize,
    accept: Vec<String>,
}

fn invalid(message: String) -> Result<Settings, ModelError> {
    Err(ModelError::Config(message))
}

/// An entry of `accept`: a dot and 1 to 16 lowercase letters or digits.
fn valid_extension(entry: &str) -> bool {
    entry.strip_prefix('.').is_some_and(|ext| {
        EXTENSION_CHARS.contains(&ext.len())
            && ext
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    })
}

fn parse(config: &Map<String, Value>) -> Result<Settings, ModelError> {
    if let Some(field) = config
        .keys()
        .find(|key| !matches!(key.as_str(), "max_files" | "accept"))
    {
        return invalid(format!(
            "unknown field `{field}`: the file type has only `max_files` and `accept`"
        ));
    }
    let max_files = match config.get("max_files") {
        None => DEFAULT_MAX_FILES,
        Some(value) => match value
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=MAX_FILES_LIMIT).contains(n))
        {
            Some(n) => n,
            None => {
                return invalid(format!(
                    "`max_files` must be a whole number from 1 to {MAX_FILES_LIMIT}"
                ));
            }
        },
    };
    let mut accept = Vec::new();
    if let Some(value) = config.get("accept") {
        let Some(list) = value.as_array() else {
            return invalid("`accept` must be a list of extensions".to_owned());
        };
        if list.len() > MAX_ACCEPT {
            return invalid(format!("`accept` has at most {MAX_ACCEPT} entries"));
        }
        for (index, entry) in list.iter().enumerate() {
            let Some(entry) = entry.as_str().filter(|entry| valid_extension(entry)) else {
                return invalid(format!(
                    "`accept[{index}]` must be a dot and 1 to 16 lowercase letters or digits, like `.pdf`"
                ));
            };
            if accept.iter().any(|seen| seen == entry) {
                return invalid(format!("`accept[{index}]` is listed twice"));
            }
            accept.push(entry.to_owned());
        }
    }
    Ok(Settings { max_files, accept })
}

pub fn validate_config(config: &Map<String, Value>) -> Result<(), ModelError> {
    parse(config).map(|_| ())
}

/// Whether a file called `file_name` may be handed in: its last extension, compared without case,
/// is in `accept`. An empty `accept` takes any name.
pub fn accepts(accept: &[String], file_name: &str) -> bool {
    if accept.is_empty() {
        return true;
    }
    let lower = file_name.to_ascii_lowercase();
    match lower.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() => accept
            .iter()
            .any(|entry| entry.strip_prefix('.') == Some(ext)),
        _ => false,
    }
}

/// The file ids of a stored content, in order. Anything that is not an id is skipped: the content
/// passed [`validate_submission`] when it was written.
fn file_ids(content: &Map<String, Value>) -> Vec<Uuid> {
    content
        .get("files")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|id| id.as_str()?.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Checks the names of the files of a content against `accept`. `names` maps a file id to its name;
/// a file with no name there cannot be checked and is refused when `accept` is set. The assessment
/// service calls this after it resolves the names with `files::get` (the type itself is pure).
pub fn check_names(
    config: &Map<String, Value>,
    content: &Map<String, Value>,
    names: &BTreeMap<Uuid, String>,
) -> Result<(), DocumentErrors> {
    let settings = parse(config)
        .map_err(|_| DocumentErrors::one("", "the file assignment is misconfigured"))?;
    let mut problems = Problems::default();
    for (index, id) in file_ids(content).iter().enumerate() {
        if !names
            .get(id)
            .is_some_and(|name| accepts(&settings.accept, name))
        {
            problems.add(
                format!("files[{index}]"),
                "is not a file type this assignment takes",
            );
        }
    }
    problems.into_result(|| ())
}

/// Checks the content of a draft or an attempt. The extension of each file is not judged here
/// (see [`check_names`]).
pub fn validate_submission(
    config: &Map<String, Value>,
    content: &Value,
    _stage: Stage,
    profiles: &EffectiveProfiles,
) -> Result<ValidContent, DocumentErrors> {
    let settings = parse(config)
        .map_err(|_| DocumentErrors::one("", "the file assignment is misconfigured"))?;
    let Some(content) = content.as_object() else {
        return Err(DocumentErrors::one("", "must be an object"));
    };
    if content.keys().any(|key| key != "files" && key != "comment") {
        return Err(DocumentErrors::one("", "has only `files` and `comment`"));
    }
    let mut problems = Problems::default();
    let mut ids = Vec::new();
    match content.get("files") {
        None => {}
        Some(Value::Array(list)) => {
            if list.len() > settings.max_files {
                problems.add("files", "has more files than the assignment takes");
            }
            let mut seen = BTreeSet::new();
            for (index, value) in list.iter().take(MAX_FILES_LIMIT + 1).enumerate() {
                match value.as_str().and_then(|text| text.parse::<Uuid>().ok()) {
                    None => problems.add(format!("files[{index}]"), "must be a file id"),
                    Some(id) if !seen.insert(id) => {
                        problems.add(format!("files[{index}]"), "is listed twice");
                    }
                    Some(id) => ids.push(id),
                }
            }
        }
        Some(_) => problems.add("files", "must be a list of file ids"),
    }
    let mut comment = None;
    if let Some(value) = content.get("comment") {
        match validate_document(value, ProfileName::Student, profiles) {
            Ok(document) if document.is_blank() => {}
            Ok(document) => comment = Some(Value::Object(document.as_body().clone())),
            Err(errors) => {
                for error in errors.iter() {
                    problems.add(
                        if error.path.is_empty() {
                            "comment".to_owned()
                        } else {
                            format!("comment.{}", error.path)
                        },
                        error.message,
                    );
                }
            }
        }
    }
    problems.into_result(|| {
        // Blank means no file: the service refuses a blank submit, a blank draft is saved (ADR-0035).
        let blank = ids.is_empty();
        let mut body = Map::new();
        body.insert(
            "files".to_owned(),
            Value::Array(ids.iter().map(|id| Value::String(id.to_string())).collect()),
        );
        if let Some(comment) = comment {
            body.insert("comment".to_owned(), comment);
        }
        ValidContent::new(body, blank)
    })
}

/// Every file a stored content names, each with its path in the content: the listed files
/// (`files[0]`) and the images and files of the comment (`comment.blocks[0].props.file_id`).
pub fn submission_file_refs(content: &Map<String, Value>) -> Result<Vec<FileRef>, DocumentErrors> {
    let mut refs: Vec<FileRef> = file_ids(content)
        .into_iter()
        .enumerate()
        .map(|(index, id)| FileRef {
            id,
            path: format!("files[{index}]"),
            image: false,
        })
        .collect();
    if let Some(comment) = content.get("comment") {
        refs.extend(file_refs_under("comment", comment)?);
    }
    Ok(refs)
}

/// The files of a content, for the service to check that the submitter may use them.
pub fn claims(_config: &Map<String, Value>, content: &Map<String, Value>) -> Claims {
    Claims {
        files: file_ids(content),
        ..Claims::default()
    }
}

#[cfg(test)]
mod tests;
