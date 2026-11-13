//! The courses of one person, as the cards, the work list and the search need them (ADR-0052).

use sqlx::PgConnection;

use super::{TeachingError, access_of};
use crate::model::{CardCourse, ScopedCourse};
use crate::repo;
use access::Actor;
use access::{self, Action};

/// The regular courses the actor studies in and may view (published or archived), by title. The
/// read interface that `gradebook` uses for `GET /me/work` (ADR-0052).
pub async fn student_courses(
    conn: &mut PgConnection,
    actor: &Actor,
) -> Result<Vec<CardCourse>, TeachingError> {
    access::authorize(actor, Action::ViewOwnCourses)?;
    let courses = repo::card_courses(conn, actor.user_id()).await?;
    Ok(courses
        .into_iter()
        .filter(|card| {
            let access = access_of(actor, &card.course, card.role);
            access.is_student() && access::authorize(actor, Action::ViewCourse(access)).is_ok()
        })
        .collect())
}

/// The regular courses the actor may view and takes part in, as staff or as a student, by title,
/// each with how the actor takes part. The number of courses of one person is small, so the list
/// is not paged. The read interface of the course cards and of search (ADR-0052).
pub async fn course_scope(
    conn: &mut PgConnection,
    actor: &Actor,
) -> Result<Vec<ScopedCourse>, TeachingError> {
    access::authorize(actor, Action::ViewOwnCourses)?;
    let courses = repo::card_courses(conn, actor.user_id()).await?;
    Ok(courses
        .into_iter()
        .filter_map(|card| {
            let access = access_of(actor, &card.course, card.role);
            access::authorize(actor, Action::ViewCourse(access)).ok()?;
            let staff = access::authorize(actor, Action::ManageContent(access)).is_ok();
            Some(ScopedCourse {
                course: card,
                owner: access.is_owner,
                staff,
                student: access.is_student(),
            })
        })
        .collect())
}
