//! Publishing, deleting and copying a course: each starts and ends in `teaching`, and in between
//! asks `content`, `assessment` and `gradebook` in the order they are stacked (ADR-0093).

use chrono::Utc;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::LearningError;
use crate::{assessment, content, gradebook};
use access::Actor;
use teaching::{self, Course, CourseTitle};

/// Moves a draft course to published. Publishing a published course changes nothing. Publishing
/// is what lets students see the assignments it was hiding, so those are announced to the
/// schedule in the same transaction (ADR-0091).
pub async fn publish(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<Course, LearningError> {
    let publication = teaching::publish_course(conn, actor, course_id).await?;
    if publication.changed {
        content::announce_visible(conn, Utc::now(), Some(course_id)).await?;
    }
    Ok(publication.course)
}

/// Deletes a course nobody has graded or submitted work in. The assignment rows are locked first
/// and the guards ask in later statements, so a grade write or a submit in flight is seen
/// (ADR-0035 point 12, ADR-0048). The course row is locked before the assignment rows, as every
/// writer of the course does.
pub async fn delete(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<(), LearningError> {
    teaching::begin_delete_course(conn, actor, course_id).await?;
    content::lock_course_assignments(conn, course_id).await?;
    if gradebook::has_grades_in_course(conn, course_id).await? {
        return Err(LearningError::CourseHasGrades);
    }
    if assessment::has_submitted_in_course(conn, course_id).await? {
        return Err(LearningError::CourseHasSubmissions);
    }
    gradebook::purge_course(conn, course_id).await?;
    assessment::purge_course(conn, course_id).await?;
    teaching::finish_delete_course(conn, actor, course_id).await?;
    Ok(())
}

/// A copy of the course for the next time: a template owned by the actor with the course's whole
/// tree as drafts.
pub async fn make_template(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    title: Option<CourseTitle>,
) -> Result<Course, LearningError> {
    let template = teaching::new_template(conn, actor, course_id, title).await?;
    let ids = content::copy_course_content(conn, course_id, template.id).await?;
    gradebook::copy_layout(conn, course_id, template.id, &ids).await?;
    Ok(template)
}

/// A draft course owned by the actor with the whole tree of a template they can edit.
pub async fn create_from_template(
    conn: &mut PgConnection,
    actor: &Actor,
    template_id: Uuid,
    title: Option<CourseTitle>,
    term_id: Option<Uuid>,
) -> Result<Course, LearningError> {
    let course =
        teaching::new_course_from_template(conn, actor, template_id, title, term_id).await?;
    let ids = content::copy_course_content(conn, template_id, course.id).await?;
    gradebook::copy_layout(conn, template_id, course.id, &ids).await?;
    Ok(course)
}
