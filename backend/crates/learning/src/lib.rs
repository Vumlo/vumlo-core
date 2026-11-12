//! The learning domain: courses and their roster, the content tree, student work and grades (ADR-0065).
//! Each submodule owns its Postgres schema and exposes only its `mod.rs`. The flows that need
//! more than one module (a person with their courses, ...) live here, above all of them.

pub mod assessment;
pub mod audit;
pub mod content;
mod enrollment;
mod error;
pub mod facts;
pub mod files;
pub mod gradebook;
pub mod groups;
pub mod people;
pub mod plugin_read;
mod points;
mod quiz_grade;
pub mod reports;
pub mod schedule;
mod state;
pub mod teaching;

pub use enrollment::event_handlers;
pub use error::LearningError;
pub use facts::rebuild_facts;
pub use state::LearningState;

/// The synchronous subscribers of the flows (ADR-0093), for the composition root to install with
/// `events::install`: each runs inside the transaction of the change that emitted its event.
pub fn subscribers() -> events::Subscribers {
    facts::subscribers().merge(quiz_grade::subscribers())
}

#[cfg(test)]
mod tests;
