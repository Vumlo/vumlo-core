//! What the administrator reads about courses (ADR-0053).

use std::collections::HashMap;

use sqlx::PgConnection;
use uuid::Uuid;

use super::TeachingError;
use crate::learning::teaching::model::{AdminCourseRow, LinkedCourseRow, UserCourseFacts};
use crate::learning::teaching::repo;
use access::Actor;
use access::{self, Action};
use common::{Page, PageParams};

/// Every regular course of the institution, paged by id. Admins only.
pub async fn admin_courses(
    conn: &mut PgConnection,
    actor: &Actor,
    page: PageParams,
) -> Result<Page<AdminCourseRow>, TeachingError> {
    access::authorize(actor, Action::ViewAdminReports)?;
    let rows = repo::admin_courses(conn, page.after, i64::from(page.limit) + 1).await?;
    Ok(Page::from_rows(rows, page.limit, |row| row.course_id))
}

/// The regular courses that use a group or one of its subgroups. The read interface `learning` uses
/// for the group overview; the caller has authorized the administrator.
pub async fn linked_courses_of_group(
    conn: &mut PgConnection,
    group_id: Uuid,
) -> Result<Vec<LinkedCourseRow>, TeachingError> {
    Ok(repo::linked_courses_of_group(conn, group_id).await?)
}

/// What a person owns, teaches and studies. The read interface `learning` uses for a person's
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

/// How many regular courses each term has; a term with none is absent. The read interface
/// `learning` uses for the term statistics; the caller has authorized the administrator.
pub async fn regular_courses_per_term(
    conn: &mut PgConnection,
) -> Result<HashMap<Uuid, i64>, TeachingError> {
    Ok(repo::regular_courses_per_term(conn)
        .await?
        .into_iter()
        .collect())
}
