//! A person seen across the institution: their account, groups and courses (ADR-0053).

mod model;
mod routes;
mod service;

pub use model::UserFacts;
pub use routes::router;
pub use service::user_facts;
