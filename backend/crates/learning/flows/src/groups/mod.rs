//! A group seen across the institution: its people and the courses that use it (ADR-0053).

mod model;
mod routes;
mod service;

pub use model::GroupOverview;
pub use routes::router;
pub use service::group_overview;
