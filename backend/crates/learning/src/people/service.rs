use sqlx::PgConnection;
use uuid::Uuid;

use super::model::UserFacts;
use crate::LearningError;
use crate::teaching;
use access::Actor;
use access::{self, Action};
use auth::{self, User, UserUpdate};
use common::Role;
use org;

/// Changes a user's name, role or status. Disabling also deletes all their sessions. The last
/// active admin can be neither demoted nor disabled, and a teacher or admin who becomes a student
/// must not own a course and stops being a course teacher everywhere (ADR-0037).
pub async fn update_user(
    conn: &mut PgConnection,
    actor: &Actor,
    user_id: Uuid,
    update: UserUpdate,
) -> Result<User, LearningError> {
    access::authorize(actor, Action::ManageUsers)?;
    let applied = auth::apply_user_update(conn, actor, user_id, update).await?;
    // This comes after the UPDATE, whose row lock waits for a course being created for this user,
    // so the check sees it; a refusal rolls the UPDATE back.
    let demoted = applied.user().role == Role::Student
        && matches!(applied.previous().role, Role::Teacher | Role::Admin);
    if demoted {
        if teaching::owns_courses(conn, user_id).await? {
            return Err(LearningError::OwnsCourses);
        }
        teaching::drop_teacher_memberships(conn, actor, user_id).await?;
    }
    Ok(applied.audit(conn, actor).await?)
}

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
