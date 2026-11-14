//! The contract of an activity type and the data its signature names (ADR-0097): the block
//! document model (ADR-0025, ADR-0027) and the stage, the validated content, the auto grade and
//! the claims of a submission. Pure code: no database, no HTTP, nothing of the other crates.

mod contract;
mod document;
mod model;
mod registry;

pub use contract::{AutoGrade, Claims, Stage, ValidContent};
pub use document::{
    DocumentError, DocumentErrors, EditorConfigError, EffectiveProfiles, FileRef, MAX_ERRORS,
    ValidDocument, distinct_ids, file_refs_of, file_refs_under, limits, plain_text, valid_href,
    validate_document, validate_document_within,
};
pub use model::{ActivityType, Capabilities, CompletionPolicy, ConfigError, Progress};
#[cfg(feature = "testing")]
pub use registry::ensure_installed;
pub use registry::{InstallError, Kind, RegistryError, Types, available, get, ids_where, install};

#[cfg(test)]
mod tests;
