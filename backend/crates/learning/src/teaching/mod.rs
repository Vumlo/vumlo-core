//! Courses, their roster and its group links (ADR-0004, ADR-0005, ADR-0019, ADR-0033).

mod model;
mod repo;
mod routes;
mod service;

pub use model::{
    CardCourse, Course, CourseGroupLink, CourseHeader, CourseInfoUpdate, CourseKind, CourseLineRow,
    CoursePeople, CourseTitle, CourseUpdate, Exclusion, LinkedCourseRow, MemberSource, Membership,
    ModelError, Person, Publication, ScopedCourse, TemplateRow, UserCourseFacts,
};
pub use routes::{api_course, api_linked_group, api_person, router};
pub use service::{
    TeachingError, admin_courses, begin_delete_course, course_access, course_exists, course_header,
    course_people, course_scope, create_course, drop_teacher_memberships, finish_delete_course,
    former_people, get_course, group_options, link_group, linked_courses_of_group, list_exclusions,
    list_links, list_members, list_my_courses, list_templates, new_course_from_template,
    new_template, owns_courses, prune_tombstones, publish_course, put_member,
    regular_courses_per_term, remove_member, restore_exclusion, student_courses,
    transfer_ownership, unlink_group, update_course, update_course_info, user_course_facts,
    user_standing_changed,
};

// Archives a course for the tests of `learning::files`.
#[cfg(test)]
pub use service::archive_course;
