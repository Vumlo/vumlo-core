//! The contract of an activity type and the data its signature names (ADR-0097): the block
//! document model (ADR-0025, ADR-0027) and the stage, the validated content, the auto grade and
//! the claims of a submission. Pure code: no database, no HTTP, nothing of the other crates.

mod contract;
pub mod document;

pub use contract::{AutoGrade, Claims, Stage, ValidContent};
pub use document::*;

#[cfg(test)]
mod tests;
