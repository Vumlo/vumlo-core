//! Business rules. Every function that acts for a user authorizes first, takes the connection it
//! runs on and never begins or commits a transaction.
//!
//! Course-scoped functions start with [`load`]: a course the actor may not view does not exist
//! as far as they can tell (404), whatever they were trying to do. Only after that does the
//! specific action decide between going ahead and 403.

use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{Course, CourseTitle, Membership, ModelError};
use super::repo;
use crate::actor::Actor;
use crate::events::{self, AuditEntry, EventsError};
use crate::identity::{self, IdentityError, Page, PageParams};
use crate::policy::{
    self, Action, CourseAccess, CourseRole, CourseStatus, PolicyError, course_role_allowed,
    implicit_course_owner,
};

#[derive(Debug, thiserror::Error)]
pub enum TeachingError {
    #[error(transparent)]
    Forbidden(#[from] PolicyError),
    #[error(transparent)]
    Invalid(#[from] ModelError),
    /// No such course, or the actor may not see it.
    #[error("course not found")]
    CourseNotFound,
    #[error("member not found")]
    MemberNotFound,
    #[error("user not found")]
    UserNotFound,
    #[error("that user cannot hold that course role")]
    RoleNotAllowed,
    #[error("the course owner must stay a teacher")]
    OwnerMustStayTeacher,
    #[error("the course owner cannot be removed")]
    OwnerCannotBeRemoved,
    #[error("an archived course cannot be published")]
    NotPublishable,
    #[error(transparent)]
    Identity(#[from] IdentityError),
    #[error(transparent)]
    Events(#[from] EventsError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

fn access_of(actor: &Actor, course: &Course, role: Option<CourseRole>) -> CourseAccess {
    CourseAccess {
        role,
        is_owner: course.owner_id == actor.user_id(),
        status: course.status,
    }
}

/// Loads the course as seen by `actor` and refuses with `CourseNotFound` unless they may view it.
/// With `lock`, the course row stays locked until the transaction ends.
async fn load(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    lock: bool,
) -> Result<(Course, CourseAccess), TeachingError> {
    let row = if lock {
        repo::lock_course(conn, course_id, actor.user_id()).await?
    } else {
        repo::find_course(conn, course_id, actor.user_id()).await?
    }
    .ok_or(TeachingError::CourseNotFound)?;
    let access = access_of(actor, &row.course, row.role);
    policy::authorize(actor, Action::ViewCourse(access))
        .map_err(|_| TeachingError::CourseNotFound)?;
    Ok((row.course, access))
}

/// Admins act as implicit owners with no membership, so everything they change in a course is
/// audited (ADR-0004).
async fn audit_admin(
    conn: &mut PgConnection,
    actor: &Actor,
    entry: AuditEntry,
) -> Result<(), TeachingError> {
    if implicit_course_owner(actor) {
        events::audit(conn, actor, entry).await?;
    }
    Ok(())
}

/// Creates a draft course owned by the actor. Teachers also join it as its `teacher`; an admin
/// owns it without a membership.
pub async fn create_course(
    conn: &mut PgConnection,
    actor: &Actor,
    title: CourseTitle,
) -> Result<Course, TeachingError> {
    policy::authorize(actor, Action::CreateCourse)?;
    let course = repo::insert_course(conn, Uuid::now_v7(), title.as_str(), actor.user_id()).await?;
    if !implicit_course_owner(actor) {
        repo::upsert_membership(conn, course.id, actor.user_id(), CourseRole::Teacher).await?;
    }
    audit_admin(
        conn,
        actor,
        AuditEntry {
            action: "course.create",
            resource_type: "course",
            resource_id: course.id,
            meta: json!({}),
        },
    )
    .await?;
    Ok(course)
}

pub async fn get_course(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<Course, TeachingError> {
    Ok(load(conn, actor, course_id, false).await?.0)
}

/// The courses the actor owns or belongs to that they may see.
pub async fn list_my_courses(
    conn: &mut PgConnection,
    actor: &Actor,
    page: PageParams,
) -> Result<Page<Course>, TeachingError> {
    policy::authorize(actor, Action::ViewOwnCourses)?;
    let rows =
        repo::list_courses_of(conn, actor.user_id(), page.after, i64::from(page.limit) + 1).await?;
    let visible = rows
        .into_iter()
        .filter(|row| {
            let access = access_of(actor, &row.course, row.role);
            policy::authorize(actor, Action::ViewCourse(access)).is_ok()
        })
        .map(|row| row.course)
        .collect();
    Ok(Page::from_rows(visible, page.limit, |course| course.id))
}

pub async fn list_members(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    page: PageParams,
) -> Result<Page<Membership>, TeachingError> {
    load(conn, actor, course_id, false).await?;
    let rows = repo::list_members(conn, course_id, page.after, i64::from(page.limit) + 1).await?;
    Ok(Page::from_rows(rows, page.limit, |member| member.user_id))
}

/// Moves a draft course to published. Publishing a published course changes nothing.
pub async fn publish_course(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<Course, TeachingError> {
    let (course, access) = load(conn, actor, course_id, true).await?;
    policy::authorize(actor, Action::PublishCourse(access))?;
    match course.status {
        CourseStatus::Published => Ok(course),
        CourseStatus::Archived => Err(TeachingError::NotPublishable),
        CourseStatus::Draft => {
            let published = repo::set_published(conn, course_id).await?;
            audit_admin(
                conn,
                actor,
                AuditEntry {
                    action: "course.publish",
                    resource_type: "course",
                    resource_id: course_id,
                    meta: json!({}),
                },
            )
            .await?;
            Ok(published)
        }
    }
}

/// Adds `user_id` to the course with `role`, or changes their role. Putting the role they
/// already have changes nothing.
///
/// Changing a role needs the right to manage both the old and the new one, so an assistant can
/// neither promote a student to teacher nor demote a teacher.
pub async fn put_member(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    user_id: Uuid,
    role: CourseRole,
) -> Result<Membership, TeachingError> {
    let (course, access) = load(conn, actor, course_id, true).await?;
    policy::authorize(actor, Action::ManageMembership { access, role })?;
    let current = repo::find_member_role(conn, course_id, user_id).await?;
    if let Some(old) = current {
        policy::authorize(actor, Action::ManageMembership { access, role: old })?;
    }
    let membership = Membership {
        course_id,
        user_id,
        role,
        is_owner: user_id == course.owner_id,
    };
    if current == Some(role) {
        return Ok(membership);
    }
    if membership.is_owner && role != CourseRole::Teacher {
        return Err(TeachingError::OwnerMustStayTeacher);
    }
    let user = identity::find_user(conn, user_id)
        .await?
        .ok_or(TeachingError::UserNotFound)?;
    if !course_role_allowed(user.role, role) {
        return Err(TeachingError::RoleNotAllowed);
    }
    repo::upsert_membership(conn, course_id, user_id, role).await?;
    audit_admin(
        conn,
        actor,
        AuditEntry {
            action: "course.member.put",
            resource_type: "course",
            resource_id: course_id,
            meta: json!({"user_id": user_id, "role": role.as_str()}),
        },
    )
    .await?;
    Ok(membership)
}

/// Removes `user_id` from the course. The owner cannot be removed.
pub async fn remove_member(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    user_id: Uuid,
) -> Result<(), TeachingError> {
    let (course, access) = load(conn, actor, course_id, true).await?;
    let role = repo::find_member_role(conn, course_id, user_id)
        .await?
        .ok_or(TeachingError::MemberNotFound)?;
    policy::authorize(actor, Action::ManageMembership { access, role })?;
    if user_id == course.owner_id {
        return Err(TeachingError::OwnerCannotBeRemoved);
    }
    repo::delete_membership(conn, course_id, user_id).await?;
    audit_admin(
        conn,
        actor,
        AuditEntry {
            action: "course.member.remove",
            resource_type: "course",
            resource_id: course_id,
            meta: json!({"user_id": user_id, "role": role.as_str()}),
        },
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::identity::{self, DisplayName, Email, HashedPassword, NewUser, Role};

    async fn actor(pool: &sqlx::PgPool, hash: &HashedPassword, email: &str, role: Role) -> Actor {
        let mut tx = pool.begin().await.unwrap();
        let user = identity::create_user(
            &mut tx,
            NewUser {
                email: Email::parse(email).unwrap(),
                display_name: DisplayName::parse("Test").unwrap(),
                role,
            },
            hash.clone(),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        Actor::new(user.id, role, Uuid::now_v7())
    }

    /// The owner promotes a student to teacher and holds its transaction open; an assistant
    /// removes that student meanwhile. The removal must wait for the course lock and then see the
    /// new role, which an assistant may not remove. Without the lock it would read the old role
    /// and delete the new teacher.
    #[sqlx::test(migrator = "crate::app::MIGRATOR")]
    async fn member_writes_on_one_course_see_each_others_committed_roles(pool: sqlx::PgPool) {
        let hash = identity::hash_password("irrelevant").await.unwrap();
        let owner = actor(&pool, &hash, "owner@x.edu", Role::Teacher).await;
        let assistant = actor(&pool, &hash, "assistant@x.edu", Role::Student).await;
        let promoted = actor(&pool, &hash, "promoted@x.edu", Role::Teacher).await;

        let mut tx = pool.begin().await.unwrap();
        let course = create_course(&mut tx, &owner, CourseTitle::parse("Algebra").unwrap())
            .await
            .unwrap();
        for (who, role) in [
            (assistant.user_id(), CourseRole::Assistant),
            (promoted.user_id(), CourseRole::Student),
        ] {
            put_member(&mut tx, &owner, course.id, who, role)
                .await
                .unwrap();
        }
        tx.commit().await.unwrap();

        let mut first = pool.begin().await.unwrap();
        put_member(
            &mut first,
            &owner,
            course.id,
            promoted.user_id(),
            CourseRole::Teacher,
        )
        .await
        .unwrap();
        let second = tokio::spawn({
            let pool = pool.clone();
            let (assistant, promoted_id, course_id) =
                (assistant.clone(), promoted.user_id(), course.id);
            async move {
                let mut tx = pool.begin().await.unwrap();
                remove_member(&mut tx, &assistant, course_id, promoted_id).await
            }
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        first.commit().await.unwrap();

        let outcome = second.await.unwrap();
        assert!(
            matches!(outcome, Err(TeachingError::Forbidden(_))),
            "{outcome:?}"
        );
        let role = repo::find_member_role(
            &mut pool.acquire().await.unwrap(),
            course.id,
            promoted.user_id(),
        )
        .await
        .unwrap();
        assert_eq!(role, Some(CourseRole::Teacher));
    }
}
