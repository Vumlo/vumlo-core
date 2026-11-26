//! The `shared_file` type (ADR-0069, ADR-0098): one uploaded file with an optional description,
//! shared with the course. It has no points and nothing is handed in; a student marks it done.
//! Pure code: whether the file may be used is checked by `learning::files::check`.

use api_types::ProfileName;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::{
    ActivityType, CompletionPolicy, ConfigError, DocumentError, DocumentErrors, EffectiveProfiles,
    FileRef, Graded, file_refs_under, validate_document,
};

pub const ID: &str = "shared_file";

/// The keys the body of a shared file may have.
const BODY_KEYS: [&str; 2] = ["file_id", "description"];

/// The file a stored body names.
fn file_id_of(body: &Map<String, Value>) -> Option<Uuid> {
    body.get("file_id")?.as_str()?.parse().ok()
}

fn problem(path: &str, message: &'static str) -> DocumentErrors {
    DocumentErrors::one(path, message)
}

/// Validates the body of a shared file: an object with at most `file_id` (a uuid) and
/// `description` (a document of the profile `full`). Returns the body to store, with the id
/// written as a plain uuid string and the description as a canonical document. Errors name paths
/// inside the body (`file_id`, `description.blocks[0]...`).
pub fn validate_body(
    body: &Value,
    profiles: &EffectiveProfiles,
) -> Result<Map<String, Value>, DocumentErrors> {
    let Value::Object(given) = body else {
        return Err(problem("", "the body of a file item must be an object"));
    };
    if let Some(unknown) = given.keys().find(|key| !BODY_KEYS.contains(&key.as_str())) {
        return Err(problem(
            unknown,
            "only `file_id` and `description` are allowed",
        ));
    }
    let mut stored = Map::new();
    if let Some(file_id) = given.get("file_id") {
        let id = file_id.as_str().and_then(|text| text.parse::<Uuid>().ok());
        let Some(id) = id else {
            return Err(problem("file_id", "must be the id of an uploaded file"));
        };
        stored.insert("file_id".to_owned(), Value::String(id.to_string()));
    }
    if let Some(description) = given.get("description") {
        let valid =
            validate_document(description, ProfileName::Full, profiles).map_err(|errors| {
                DocumentErrors::of(
                    errors
                        .iter()
                        .map(|error| DocumentError {
                            path: if error.path.is_empty() {
                                "description".to_owned()
                            } else {
                                format!("description.{}", error.path)
                            },
                            message: error.message,
                        })
                        .collect(),
                )
            })?;
        stored.insert(
            "description".to_owned(),
            Value::Object(valid.as_body().clone()),
        );
    }
    Ok(stored)
}

pub struct SharedFile;

impl ActivityType for SharedFile {
    fn id(&self) -> &'static str {
        ID
    }

    fn completion(&self) -> CompletionPolicy {
        CompletionPolicy::Marked
    }

    fn graded(&self) -> Option<&dyn Graded> {
        None
    }

    fn default_body(&self) -> Map<String, Value> {
        Map::new()
    }

    fn validate_body(
        &self,
        body: &Value,
        profiles: &EffectiveProfiles,
    ) -> Result<Map<String, Value>, DocumentErrors> {
        validate_body(body, profiles)
    }

    fn body_file_refs(&self, body: &Map<String, Value>) -> Result<Vec<FileRef>, DocumentErrors> {
        let mut refs = Vec::new();
        if let Some(id) = file_id_of(body) {
            refs.push(FileRef {
                id,
                path: "body.file_id".to_owned(),
                image: false,
            });
        }
        if let Some(description) = body.get("description") {
            refs.extend(file_refs_under("body.description", description)?);
        }
        Ok(refs)
    }

    fn check_publishable(&self, body: &Map<String, Value>) -> Result<(), ConfigError> {
        if file_id_of(body).is_some() {
            Ok(())
        } else {
            Err(ConfigError::new(
                "a file item needs a file before it is published",
            ))
        }
    }
}

#[cfg(test)]
mod tests;
