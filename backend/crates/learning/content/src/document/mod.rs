//! Block documents (ADR-0025, ADR-0027): the validator, the profiles and `plain_text`. The types
//! live in `api-types`; this module turns a JSON value into one and checks it against a profile.

mod parse;
mod profile;
mod text;

use std::collections::BTreeSet;

use api_types::{Block, Document, DocumentLimits, Feature, FieldError, ProfileName};
use serde_json::{Map, Value};
use uuid::Uuid;

pub(crate) use parse::valid_href;
pub use profile::{EditorConfigError, EffectiveProfiles};
pub use text::plain_text;

/// Serialized size of a whole document.
pub const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;
/// Blocks nest at most this deep; the top level is level 1.
pub const MAX_DEPTH: usize = 6;
pub const MAX_TABLE_ROWS: usize = 100;
pub const MAX_TABLE_COLUMNS: usize = 20;
pub const MAX_CODE_BYTES: usize = 64 * 1024;
/// A formula, inline or in a block, or a diagram.
pub const MAX_FORMULA_BYTES: usize = 16 * 1024;
pub const MAX_HREF_CHARS: usize = 2048;
pub const MAX_ID_CHARS: usize = 64;
/// The alt text and the caption of an image.
pub const MAX_IMAGE_TEXT_CHARS: usize = 300;
/// More problems than this are not reported.
pub const MAX_ERRORS: usize = 20;

/// The limits as the API shows them.
pub fn limits() -> DocumentLimits {
    // The limits are small constants, so the conversions cannot overflow `u32`.
    let n = |value: usize| u32::try_from(value).unwrap_or(u32::MAX);
    DocumentLimits {
        max_document_bytes: n(MAX_DOCUMENT_BYTES),
        max_depth: n(MAX_DEPTH),
        max_table_rows: n(MAX_TABLE_ROWS),
        max_table_columns: n(MAX_TABLE_COLUMNS),
        max_code_bytes: n(MAX_CODE_BYTES),
        max_formula_bytes: n(MAX_FORMULA_BYTES),
        max_href_chars: n(MAX_HREF_CHARS),
    }
}

/// One problem. `path` runs from the document, for example `blocks[2].props.level`; it is empty
/// for the document itself. The message is fixed text and never contains what was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentError {
    pub path: String,
    pub message: &'static str,
}

/// Every problem found in one document, at most [`MAX_ERRORS`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentErrors(Vec<DocumentError>);

impl DocumentErrors {
    pub fn iter(&self) -> impl Iterator<Item = &DocumentError> {
        self.0.iter()
    }

    /// The errors as API field errors under `field`, for example `body.blocks[2].props.level`.
    pub fn into_field_errors(self, field: &str) -> Vec<FieldError> {
        self.0
            .into_iter()
            .map(|error| FieldError {
                field: if error.path.is_empty() {
                    field.to_owned()
                } else {
                    format!("{field}.{}", error.path)
                },
                message: error.message.to_owned(),
            })
            .collect()
    }

    pub(crate) fn of(errors: Vec<DocumentError>) -> Self {
        Self(errors)
    }

    /// One problem at `path` (an empty path is the document itself).
    pub fn one(path: &str, message: &'static str) -> Self {
        Self(vec![DocumentError {
            path: path.to_owned(),
            message,
        }])
    }
}

/// A document that passed [`validate_document`]. Only the validator builds one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidDocument {
    document: Document,
    body: Map<String, Value>,
}

impl ValidDocument {
    /// The document of an item that has no text yet.
    pub fn empty() -> Self {
        let document = Document::empty();
        let body = match serde_json::to_value(&document) {
            Ok(Value::Object(body)) => body,
            _ => Map::new(),
        };
        Self { document, body }
    }

    /// True when the document holds no text, image or file (ADR-0035): an empty draft may be
    /// saved, but never submitted.
    pub fn is_blank(&self) -> bool {
        plain_text(&self.document).trim().is_empty() && file_ids(&self.document).is_empty()
    }

    pub fn as_document(&self) -> &Document {
        &self.document
    }

    /// The stored form: the canonical serialization of the document.
    pub fn as_body(&self) -> &Map<String, Value> {
        &self.body
    }
}

/// A file a document names: its id, the path of its `file_id` in the document
/// (`blocks[2].children[0].props.file_id`) and whether an `image` block (and not a `file` block)
/// names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRef {
    pub id: Uuid,
    pub path: String,
    pub image: bool,
}

/// The files the image and file blocks of a document name, in document order. A file named twice
/// appears twice.
pub fn file_refs(document: &Document) -> Vec<FileRef> {
    fn walk(blocks: &[Block], path: &str, out: &mut Vec<FileRef>) {
        for (index, block) in blocks.iter().enumerate() {
            let here = format!("{path}[{index}]");
            let named = match block {
                Block::Image { props, .. } => Some((props.file_id, true)),
                Block::File { props, .. } => Some((props.file_id, false)),
                _ => None,
            };
            if let Some((id, image)) = named {
                out.push(FileRef {
                    id,
                    path: format!("{here}.props.file_id"),
                    image,
                });
            }
            walk(block.children(), &format!("{here}.children"), out);
        }
    }
    let mut out = Vec::new();
    walk(&document.blocks, "blocks", &mut out);
    out
}

/// The ids of the files a document names, in document order, each once (ADR-0069).
pub fn file_ids(document: &Document) -> Vec<Uuid> {
    distinct_ids(&file_refs(document))
}

/// The ids of `refs`, each once, in order.
pub fn distinct_ids(refs: &[FileRef]) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = Vec::new();
    for file in refs {
        if !ids.contains(&file.id) {
            ids.push(file.id);
        }
    }
    ids
}

/// [`file_refs`] of a stored or submitted document found at `prefix` (a path such as
/// `answers.q1`, empty for the document itself), read by the validator's own parser so that the
/// two can never disagree about what the document holds. The paths of the references and of the
/// errors start with `prefix`. A value the parser refuses is an error, never "no files": a
/// reference that is skipped would be stored unchecked and unrecorded.
pub fn file_refs_under(prefix: &str, value: &Value) -> Result<Vec<FileRef>, DocumentErrors> {
    let join = |path: &str| match (prefix.is_empty(), path.is_empty()) {
        (true, _) => path.to_owned(),
        (false, true) => prefix.to_owned(),
        (false, false) => format!("{prefix}.{path}"),
    };
    let (document, errors) = parse::parse_document(value);
    match document {
        Some(document) if errors.is_empty() => Ok(file_refs(&document)
            .into_iter()
            .map(|file| FileRef {
                path: join(&file.path),
                ..file
            })
            .collect()),
        _ => Err(DocumentErrors::of(
            errors
                .into_iter()
                .map(|error| DocumentError {
                    path: join(&error.path),
                    message: error.message,
                })
                .collect(),
        )),
    }
}

/// [`file_refs_under`] the document itself.
pub fn file_refs_of(value: &Value) -> Result<Vec<FileRef>, DocumentErrors> {
    file_refs_under("", value)
}

/// Parses `value` into a [`Document`] and checks it against `profile` as narrowed by `profiles`.
///
/// Reports every problem at once (at most [`MAX_ERRORS`]) with its path. The profile is checked
/// only when the shape is sound, because it walks the typed document.
pub fn validate_document(
    value: &Value,
    profile: ProfileName,
    profiles: &EffectiveProfiles,
) -> Result<ValidDocument, DocumentErrors> {
    validate_document_within(value, profile, profiles, None)
}

/// Like [`validate_document`], with a third layer that can only narrow (ADR-0025): when `within`
/// is set, only the features of the profile that it also lists are allowed. A feature it lists
/// that the profile lacks widens nothing.
pub fn validate_document_within(
    value: &Value,
    profile: ProfileName,
    profiles: &EffectiveProfiles,
    within: Option<&BTreeSet<Feature>>,
) -> Result<ValidDocument, DocumentErrors> {
    // A value that is already in memory always serializes, so the fallback is a defensive
    // refusal of an absurd size rather than a case that happens.
    let size = serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len());
    if size > MAX_DOCUMENT_BYTES {
        return Err(DocumentErrors::one("", "is larger than 1 MiB"));
    }
    let (document, errors) = parse::parse_document(value);
    let Some(document) = document.filter(|_| errors.is_empty()) else {
        return Err(DocumentErrors::of(errors));
    };
    let mut effective = profiles.profile(profile).clone();
    if let Some(within) = within {
        effective
            .features
            .retain(|feature| within.contains(feature));
    }
    let errors = profile::check_profile(&document, &effective, profiles);
    if !errors.is_empty() {
        return Err(DocumentErrors::of(errors));
    }
    match serde_json::to_value(&document) {
        Ok(Value::Object(body)) => Ok(ValidDocument { document, body }),
        _ => Err(DocumentErrors::one("", "cannot be stored")),
    }
}

#[cfg(test)]
mod tests;
