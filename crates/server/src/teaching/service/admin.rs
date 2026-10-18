//! What the administrator reads about courses (ADR-0053).

use sqlx::PgConnection;
use uuid::Uuid;

use super::TeachingError;
use crate::actor::Actor;
use crate::pagination::{Page, PageParams};
use crate::policy::{self, Action};
use crate::teaching::model::{AdminCourseRow, LinkedCourseRow, UserCourseFacts};
use crate::teaching::repo;

/// Every regular course of the institution, paged by id. Admins only.
pub async fn admin_courses(
    conn: &mut PgConnection,
    actor: &Actor,
    page: PageParams,
) -> Result<Page<AdminCourseRow>, TeachingError> {
    policy::authorize(actor, Action::ViewAdminReports)?;
    let rows = repo::admin_courses(conn, page.after, i64::from(page.limit) + 1).await?;
    Ok(Page::from_rows(rows, page.limit, |row| row.course_id))
}

/// The regular courses that use a group or one of its subgroups. The read interface `org` uses
/// for the group overview; the caller has authorized the administrator.
pub async fn linked_courses_of_group(
    conn: &mut PgConnection,
    group_id: Uuid,
) -> Result<Vec<LinkedCourseRow>, TeachingError> {
    Ok(repo::linked_courses_of_group(conn, group_id).await?)
}

/// What a person owns, teaches and studies. The read interface `identity` uses for a person's
/// facts; the caller has authorized the administrator.
pub async fn user_course_facts(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<UserCourseFacts, TeachingError> {
    Ok(UserCourseFacts {
        owns: repo::owned_courses(conn, user_id).await?,
        teaches: repo::taught_courses(conn, user_id).await?,
        studies: repo::studied_courses(conn, user_id).await?,
    })
}
