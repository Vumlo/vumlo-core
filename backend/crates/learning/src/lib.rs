//! The learning domain: courses and their roster, the content tree, student work and grades (ADR-0065).
//! Each submodule owns its Postgres schema and exposes only its `mod.rs`. The flows that need
//! more than one module (a person with their courses, ...) live here, above all of them.

pub mod assessment;
pub mod audit;
pub mod content;
mod enrollment;
mod error;
pub mod files;
pub mod gradebook;
pub mod groups;
pub mod people;
pub mod plugin_read;
mod points;
pub mod reports;
mod state;
pub mod teaching;

pub use enrollment::event_handlers;
pub use error::LearningError;
pub use state::LearningState;
