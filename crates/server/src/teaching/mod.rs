//! Courses and their members (ADR-0004, ADR-0005, ADR-0019).

mod model;
mod repo;
mod routes;
mod service;

pub use model::{Course, CourseKind, CourseTitle, Membership, ModelError};
pub use routes::router;
pub use service::{
    TeachingError, course_access, create_course, get_course, list_members, list_my_courses,
    publish_course, put_member, remove_member,
};
