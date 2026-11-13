//! What needs several modules about courses: the cards of the home page (ADR-0052), the header and
//! the people of a course, and its lifecycle (ADR-0048). Each is a flow above `teaching`,
//! `content`, `assessment` and `gradebook`, which are asked in that order and never ask back
//! (ADR-0093).

mod model;
mod routes;
mod service;

pub use model::{CourseCardData, CourseInfo, Template, Todo};
pub use routes::router;
pub use service::{
    cards, create_from_template, delete, info, make_template, people, publish, templates,
    update_info,
};
