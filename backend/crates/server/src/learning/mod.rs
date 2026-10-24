//! The learning domain: courses and their roster, the content tree, student work and grades (ADR-0065).
//! Each submodule owns its Postgres schema and exposes only its `mod.rs`.

pub mod assessment;
pub mod content;
pub mod gradebook;
mod points;
pub mod teaching;
