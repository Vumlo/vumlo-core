//! The facts the access rules read (ADR-0093). `content` keeps a copy of what `assessment` and
//! `gradebook` say about a student on an assignment: when they first submitted and which score they
//! were given. A flow here, above all three, copies it: the subscribers do it for one student and
//! one assignment in the transaction of the change, and [`rebuild_facts`] does it again for a course
//! or for everything, so the copy can always be made from the sources.

mod repo;
mod service;

pub use service::{rebuild_facts, subscribers, sync};
