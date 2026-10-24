//! The learning domain: courses and their roster, the content tree, student work and grades (ADR-0065).
//! Each submodule owns its Postgres schema and exposes only its `mod.rs`. The flows that need
//! more than one module (a person with their courses, ...) live here, above all of them.

pub mod assessment;
pub mod content;
mod error;
pub mod gradebook;
pub mod people;
mod points;
pub mod teaching;

pub use error::LearningError;
