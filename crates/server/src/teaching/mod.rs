//! Courses, their roster and its group links (ADR-0004, ADR-0005, ADR-0019, ADR-0033).

mod model;
mod repo;
mod routes;
mod service;

pub use model::{
    Course, CourseGroupLink, CourseKind, CourseTitle, CourseUpdate, Exclusion, MemberSource,
    Membership, ModelError,
};
pub use routes::router;
pub use service::{
    TeachingError, course_access, create_course, get_course, group_options, link_group,
    list_exclusions, list_links, list_members, list_my_courses, publish_course, put_member,
    remove_member, restore_exclusion, unlink_group, update_course,
};
