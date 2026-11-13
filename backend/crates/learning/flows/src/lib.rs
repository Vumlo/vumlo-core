//! The flows of the learning domain (ADR-0065, ADR-0093): what needs several of `teaching`,
//! `content`, `assessment` and `gradebook` (a person with their courses, a course's cards and
//! lifecycle, items, submissions, the file check, the facts, plugin reads, the scheduler). It owns
//! no schema; each submodule exposes only its `mod.rs`.

pub mod audit;
pub mod courses;
mod enrollment;
mod error;
pub mod facts;
pub mod files;
pub mod groups;
pub mod items;
pub mod people;
pub mod plugin_read;
mod quiz_grade;
pub mod reports;
pub mod schedule;
mod state;
pub mod submissions;

pub use enrollment::event_handlers;
pub use error::LearningError;
pub use facts::rebuild_facts;
pub use state::LearningState;

/// The synchronous subscribers of the flows (ADR-0093), for the composition root to install with
/// `events::install`: each runs inside the transaction of the change that emitted its event.
/// The order is the order of the locks: a submit takes the grade row (the quiz grade) before the
/// facts of the pair, as publishing a grade does.
pub fn subscribers() -> events::Subscribers {
    quiz_grade::subscribers().merge(facts::subscribers())
}

#[cfg(test)]
mod tests;
