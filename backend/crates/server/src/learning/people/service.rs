use sqlx::PgConnection;
use uuid::Uuid;

use super::model::UserFacts;
use crate::access::{self, Action};
use crate::actor::Actor;
use crate::auth;
use crate::learning::LearningError;
use crate::learning::teaching;
use crate::org;

/// What one person does in the institution: groups, courses, last sign-in. Admins only
/// (ADR-0053).
pub async fn user_facts(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<UserFacts, LearningError> {
    access::authorize(actor, Action::ViewAdminReports)?;
    let user = auth::find_user(conn, id)
        .await?
        .ok_or(auth::AuthError::UserNotFound)?;
    let last_login_at = auth::last_login_at(conn, id).await?;
    let groups = org::groups_of_user(conn, id).await?;
    let courses = teaching::user_course_facts(conn, id).await?;
    Ok(UserFacts {
        user,
        last_login_at,
        groups,
        courses,
    })
}

#[cfg(test)]
mod tests;
