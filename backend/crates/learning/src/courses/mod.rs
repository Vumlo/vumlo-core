//! What needs several modules about courses: the cards of the home page (ADR-0052). Each is a flow
//! above `teaching`, `content` and `gradebook`, which are asked in that order and never ask back
//! (ADR-0093).

mod model;
mod routes;
mod service;

pub use model::{CourseCardData, Todo};
pub use routes::router;
pub use service::cards;
