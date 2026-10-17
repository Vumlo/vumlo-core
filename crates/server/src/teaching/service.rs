//! Business rules. Every function that acts for a user authorizes first, takes the connection it
//! runs on and never begins or commits a transaction.
//!
//! Course-scoped functions start with [`load`]: a course the actor may not view does not exist
//! as far as they can tell (404), whatever they were trying to do. Only after that does the
//! specific action decide between going ahead and 403.

use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{
    Course, CourseGroupLink, CourseKind, CourseTitle, CourseUpdate, Exclusion, MemberSource,
    Membership, ModelError,
};
use super::repo;
use crate::actor::Actor;
use crate::events::{self, AuditEntry, EventsError};
use crate::identity::{self, IdentityError, UserStatus};
use crate::org::{self, GroupOption, OrgError};
use crate::pagination::{Page, PageParams};
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
    /// The `term_id` of a new or edited course names no term.
    #[error("term not found")]
    TermNotFound,
    #[error("a template has no term")]
    TemplateHasNoTerm,
    #[error("link not found")]
    LinkNotFound,
    #[error("exclusion not found")]
    ExclusionNotFound,
    /// The `group_id` of a new link names no group.
    #[error("group not found")]
    GroupNotFound,
    /// The `subgroup_id` of a new link names no subgroup of that group.
    #[error("subgroup not found in that group")]
    SubgroupNotValid,
    #[error("that user is disabled")]
    UserDisabled,
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
    Org(#[from] OrgError),
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
/// The actor's role is their place on the roster, the view `teaching.effective_members`: a
/// direct membership, or a student through a linked group (ADR-0033).
///
/// With `lock`, the course row stays locked until the transaction ends, and the actor's role is
/// read in a second statement after the lock is held. One statement joining both would wait for
/// the lock and then re-read only the course row, leaving the role as of the statement's
/// snapshot: an assistant demoted by the writer we waited for would still act as an assistant.
/// Without `lock` nothing waits, so one statement is enough.
async fn load(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    lock: bool,
) -> Result<(Course, CourseAccess), TeachingError> {
    let (course, role) = if lock {
        let course = repo::lock_course(conn, course_id)
            .await?
            .ok_or(TeachingError::CourseNotFound)?;
        let role = repo::find_member(conn, course_id, actor.user_id())
            .await?
            .map(|entry| entry.role);
        (course, role)
    } else {
        let row = repo::find_course(conn, course_id, actor.user_id())
            .await?
            .ok_or(TeachingError::CourseNotFound)?;
        (row.course, row.role)
    };
    let access = access_of(actor, &course, role);
    policy::authorize(actor, Action::ViewCourse(access))
        .map_err(|_| TeachingError::CourseNotFound)?;
    Ok((course, access))
}

/// An unknown `term_id` violates the foreign key to `org.terms`.
fn term_fk(error: sqlx::Error) -> TeachingError {
    match &error {
        sqlx::Error::Database(db)
            if db.is_foreign_key_violation() && db.constraint() == Some("courses_term_id_fkey") =>
        {
            TeachingError::TermNotFound
        }
        _ => TeachingError::Db(error),
    }
}

/// Creates a draft course owned by the actor. Teachers also join it as its `teacher`; an admin
/// owns it without a membership.
pub async fn create_course(
    conn: &mut PgConnection,
    actor: &Actor,
    title: CourseTitle,
    term_id: Option<Uuid>,
) -> Result<Course, TeachingError> {
    policy::authorize(actor, Action::CreateCourse)?;
    let course = repo::insert_course(
        conn,
        Uuid::now_v7(),
        title.as_str(),
        actor.user_id(),
        term_id,
    )
    .await
    .map_err(term_fk)?;
    if !implicit_course_owner(actor) {
        repo::upsert_membership(conn, course.id, actor.user_id(), CourseRole::Teacher).await?;
    }
    events::audit_admin(
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

/// How `actor` relates to the course, for other modules that authorize their own actions
/// against it. A course the actor may not view is `CourseNotFound`.
pub async fn course_access(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<CourseAccess, TeachingError> {
    Ok(load(conn, actor, course_id, false).await?.1)
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
            events::audit_admin(
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
/// already have on the roster changes nothing, also for a student who comes from a linked group.
///
/// Changing a role needs the right to manage both the old and the new one, so an assistant can
/// neither promote a student to teacher nor demote a teacher. A direct membership clears the
/// user's exclusion (ADR-0033).
pub async fn put_member(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    user_id: Uuid,
    role: CourseRole,
) -> Result<Membership, TeachingError> {
    let (course, access) = load(conn, actor, course_id, true).await?;
    policy::authorize(actor, Action::ManageMembership { access, role })?;
    let current = repo::find_member(conn, course_id, user_id).await?;
    if let Some(old) = current {
        policy::authorize(
            actor,
            Action::ManageMembership {
                access,
                role: old.role,
            },
        )?;
    }
    let membership = |source| Membership {
        course_id,
        user_id,
        role,
        is_owner: user_id == course.owner_id,
        source,
    };
    if let Some(old) = current.filter(|old| old.role == role) {
        return Ok(membership(old.source));
    }
    if user_id == course.owner_id && role != CourseRole::Teacher {
        return Err(TeachingError::OwnerMustStayTeacher);
    }
    let user = identity::find_user(conn, user_id)
        .await?
        .ok_or(TeachingError::UserNotFound)?;
    if user.status == UserStatus::Disabled {
        return Err(TeachingError::UserDisabled);
    }
    if !course_role_allowed(user.role, role) {
        return Err(TeachingError::RoleNotAllowed);
    }
    repo::upsert_membership(conn, course_id, user_id, role).await?;
    repo::delete_exclusion(conn, course_id, user_id).await?;
    events::audit_admin(
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
    Ok(membership(MemberSource::Direct))
}

/// Removes `user_id` from the roster of the course. The owner cannot be removed.
///
/// A student who comes from a linked group, or a direct student a linked group would still put on
/// the roster, is excluded from this course (ADR-0033). Removing a direct teacher or assistant
/// deletes the direct row only.
pub async fn remove_member(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    user_id: Uuid,
) -> Result<(), TeachingError> {
    let (course, access) = load(conn, actor, course_id, true).await?;
    let member = repo::find_member(conn, course_id, user_id)
        .await?
        .ok_or(TeachingError::MemberNotFound)?;
    policy::authorize(
        actor,
        Action::ManageMembership {
            access,
            role: member.role,
        },
    )?;
    if user_id == course.owner_id {
        return Err(TeachingError::OwnerCannotBeRemoved);
    }
    let excluded = match (member.source, member.role) {
        (MemberSource::Group, _) => true,
        (MemberSource::Direct, role) => {
            repo::delete_membership(conn, course_id, user_id).await?;
            role == CourseRole::Student && repo::in_group_roster(conn, course_id, user_id).await?
        }
    };
    if excluded {
        repo::insert_exclusion(conn, course_id, user_id, actor.user_id()).await?;
    }
    events::audit_admin(
        conn,
        actor,
        AuditEntry {
            action: "course.member.remove",
            resource_type: "course",
            resource_id: course_id,
            meta: json!({"user_id": user_id, "role": member.role.as_str(), "excluded": excluded}),
        },
    )
    .await?;
    Ok(())
}

/// Changes the title and/or the term of a course (ADR-0033). Staff and admins, whatever the
/// status. A field that is absent changes nothing; `term_id: null` clears the term.
pub async fn update_course(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    update: CourseUpdate,
) -> Result<Course, TeachingError> {
    let (course, access) = load(conn, actor, course_id, true).await?;
    policy::authorize(actor, Action::EditCourse(access))?;
    let title = update
        .title
        .as_deref()
        .map(CourseTitle::parse)
        .transpose()?;
    if matches!(update.term_id, Some(Some(_))) && course.kind == CourseKind::Template {
        return Err(TeachingError::TemplateHasNoTerm);
    }
    if title.is_none() && update.term_id.is_none() {
        return Ok(course);
    }
    let mut fields = Vec::new();
    if title.is_some() {
        fields.push("title");
    }
    if update.term_id.is_some() {
        fields.push("term_id");
    }
    let updated = repo::update_course(
        conn,
        course_id,
        title.as_ref().map(CourseTitle::as_str),
        update.term_id.is_some(),
        update.term_id.flatten(),
    )
    .await
    .map_err(term_fk)?;
    events::audit_admin(
        conn,
        actor,
        AuditEntry {
            action: "course.update",
            resource_type: "course",
            resource_id: course_id,
            meta: json!({"fields": fields}),
        },
    )
    .await?;
    Ok(updated)
}

// ---------------------------------------------------------------------------------------------
// Group links and exclusions (ADR-0033)
// ---------------------------------------------------------------------------------------------

/// The groups and subgroups linked to the course, in link-id order.
pub async fn list_links(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    page: PageParams,
) -> Result<Page<CourseGroupLink>, TeachingError> {
    let (_, access) = load(conn, actor, course_id, false).await?;
    policy::authorize(actor, Action::ManageRoster(access))?;
    let rows = repo::list_links(conn, course_id, page.after, i64::from(page.limit) + 1).await?;
    Ok(Page::from_rows(rows, page.limit, |link| link.id))
}

/// Links the course to a group, or to one subgroup of it. The second value is `false` when the
/// link existed already (the same link comes back). The foreign keys decide that the group
/// exists and that the subgroup belongs to it, so there is no race with a deletion in `org`.
pub async fn link_group(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    group_id: Uuid,
    subgroup_id: Option<Uuid>,
) -> Result<(CourseGroupLink, bool), TeachingError> {
    let (_, access) = load(conn, actor, course_id, true).await?;
    policy::authorize(actor, Action::ManageRoster(access))?;
    let inserted = repo::insert_link(
        conn,
        Uuid::now_v7(),
        course_id,
        group_id,
        subgroup_id,
        actor.user_id(),
    )
    .await
    .map_err(|error| match &error {
        sqlx::Error::Database(db) if db.is_foreign_key_violation() => match db.constraint() {
            Some("course_groups_group_id_fkey") => TeachingError::GroupNotFound,
            Some("course_groups_subgroup_fkey") => TeachingError::SubgroupNotValid,
            _ => TeachingError::Db(error),
        },
        _ => TeachingError::Db(error),
    })?;
    let Some(link) = inserted else {
        let existing = repo::find_link_by_target(conn, course_id, group_id, subgroup_id)
            .await?
            .ok_or(TeachingError::LinkNotFound)?;
        return Ok((existing, false));
    };
    events::audit_admin(
        conn,
        actor,
        AuditEntry {
            action: "course.group.link",
            resource_type: "course",
            resource_id: course_id,
            meta: json!({"group_id": group_id, "subgroup_id": subgroup_id}),
        },
    )
    .await?;
    Ok((link, true))
}

pub async fn unlink_group(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    link_id: Uuid,
) -> Result<(), TeachingError> {
    let (_, access) = load(conn, actor, course_id, true).await?;
    policy::authorize(actor, Action::ManageRoster(access))?;
    let link = repo::delete_link(conn, course_id, link_id)
        .await?
        .ok_or(TeachingError::LinkNotFound)?;
    events::audit_admin(
        conn,
        actor,
        AuditEntry {
            action: "course.group.unlink",
            resource_type: "course",
            resource_id: course_id,
            meta: json!({"group_id": link.group_id, "subgroup_id": link.subgroup_id}),
        },
    )
    .await?;
    Ok(())
}

/// The groups with their subgroups (ids and names) a course can be linked to, for those who may
/// manage its roster. `org` answers; its own group reads stay admin only.
pub async fn group_options(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    page: PageParams,
) -> Result<Page<GroupOption>, TeachingError> {
    let (_, access) = load(conn, actor, course_id, false).await?;
    policy::authorize(actor, Action::ManageRoster(access))?;
    Ok(org::group_options(conn, page).await?)
}

pub async fn list_exclusions(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    page: PageParams,
) -> Result<Page<Exclusion>, TeachingError> {
    let (_, access) = load(conn, actor, course_id, false).await?;
    policy::authorize(actor, Action::ManageRoster(access))?;
    let rows =
        repo::list_exclusions(conn, course_id, page.after, i64::from(page.limit) + 1).await?;
    Ok(Page::from_rows(rows, page.limit, |exclusion| {
        exclusion.user_id
    }))
}

/// Deletes the exclusion: a student of a linked group is on the roster again.
pub async fn restore_exclusion(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    user_id: Uuid,
) -> Result<(), TeachingError> {
    let (_, access) = load(conn, actor, course_id, true).await?;
    policy::authorize(actor, Action::ManageRoster(access))?;
    if repo::delete_exclusion(conn, course_id, user_id).await? == 0 {
        return Err(TeachingError::ExclusionNotFound);
    }
    events::audit_admin(
        conn,
        actor,
        AuditEntry {
            action: "course.exclusion.restore",
            resource_type: "course",
            resource_id: course_id,
            meta: json!({"user_id": user_id}),
        },
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::test_support::{db_test, wait_until_blocked};

    use super::*;
    use crate::identity::{self, Email, HashedPassword, NewUser, PersonName, Role};

    async fn actor(pool: &sqlx::PgPool, hash: &HashedPassword, email: &str, role: Role) -> Actor {
        let mut tx = pool.begin().await.unwrap();
        let user = identity::create_user_unaudited_for_tests(
            &mut tx,
            NewUser {
                email: Email::parse(email).unwrap(),
                name: PersonName::name("Test").unwrap(),
                surname: PersonName::surname("Test").unwrap(),
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
    #[tokio::test]
    async fn member_writes_on_one_course_see_each_others_committed_roles() {
        db_test(async |pool| {
            let hash = identity::hash_password("irrelevant").await.unwrap();
            let owner = actor(&pool, &hash, "owner@x.edu", Role::Teacher).await;
            let assistant = actor(&pool, &hash, "assistant@x.edu", Role::Student).await;
            let promoted = actor(&pool, &hash, "promoted@x.edu", Role::Teacher).await;

            let mut tx = pool.begin().await.unwrap();
            let course = create_course(
                &mut tx,
                &owner,
                CourseTitle::parse("Algebra").unwrap(),
                None,
            )
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
            wait_until_blocked(&pool, &second).await;
            first.commit().await.unwrap();

            let outcome = second.await.unwrap();
            assert!(
                matches!(outcome, Err(TeachingError::Forbidden(_))),
                "{outcome:?}"
            );
            let role = repo::find_member(
                &mut pool.acquire().await.unwrap(),
                course.id,
                promoted.user_id(),
            )
            .await
            .unwrap()
            .map(|entry| entry.role);
            assert_eq!(role, Some(CourseRole::Teacher));
        })
        .await;
    }

    /// The owner demotes an assistant to student and holds its transaction open; the assistant
    /// tries to add a student meanwhile. The assistant's own role must be read after the course
    /// lock is held: a role read in the same statement as the lock would still be the old one.
    /// The course is published, otherwise the demoted assistant could no longer see it and the
    /// answer would be `CourseNotFound` instead of `Forbidden`.
    #[tokio::test]
    async fn a_demoted_assistant_loses_their_rights_before_their_waiting_write_runs() {
        db_test(async |pool| {
            let hash = identity::hash_password("irrelevant").await.unwrap();
            let owner = actor(&pool, &hash, "owner@x.edu", Role::Teacher).await;
            let assistant = actor(&pool, &hash, "assistant@x.edu", Role::Student).await;
            let enrolled = actor(&pool, &hash, "enrolled@x.edu", Role::Student).await;
            let newcomer = actor(&pool, &hash, "newcomer@x.edu", Role::Student).await;

            let mut tx = pool.begin().await.unwrap();
            let course = create_course(
                &mut tx,
                &owner,
                CourseTitle::parse("Algebra").unwrap(),
                None,
            )
            .await
            .unwrap();
            for (who, role) in [
                (assistant.user_id(), CourseRole::Assistant),
                (enrolled.user_id(), CourseRole::Student),
            ] {
                put_member(&mut tx, &owner, course.id, who, role)
                    .await
                    .unwrap();
            }
            publish_course(&mut tx, &owner, course.id).await.unwrap();
            tx.commit().await.unwrap();

            let mut first = pool.begin().await.unwrap();
            put_member(
                &mut first,
                &owner,
                course.id,
                assistant.user_id(),
                CourseRole::Student,
            )
            .await
            .unwrap();
            let second = tokio::spawn({
                let pool = pool.clone();
                let (assistant, newcomer_id, course_id) =
                    (assistant.clone(), newcomer.user_id(), course.id);
                async move {
                    let mut tx = pool.begin().await.unwrap();
                    put_member(
                        &mut tx,
                        &assistant,
                        course_id,
                        newcomer_id,
                        CourseRole::Student,
                    )
                    .await
                }
            });
            wait_until_blocked(&pool, &second).await;
            first.commit().await.unwrap();

            let outcome = second.await.unwrap();
            assert!(
                matches!(outcome, Err(TeachingError::Forbidden(_))),
                "{outcome:?}"
            );
            let mut conn = pool.acquire().await.unwrap();
            let assistant_role = repo::find_member(&mut conn, course.id, assistant.user_id())
                .await
                .unwrap()
                .map(|entry| entry.role);
            assert_eq!(assistant_role, Some(CourseRole::Student));
            let newcomer_role = repo::find_member(&mut conn, course.id, newcomer.user_id())
                .await
                .unwrap()
                .map(|entry| entry.role);
            assert_eq!(newcomer_role, None);
        })
        .await;
    }

    #[tokio::test]
    async fn course_access_describes_the_actor_and_hides_courses_they_cannot_view() {
        db_test(async |pool| {
            let hash = identity::hash_password("irrelevant").await.unwrap();
            let owner = actor(&pool, &hash, "owner@x.edu", Role::Teacher).await;
            let admin = actor(&pool, &hash, "admin@x.edu", Role::Admin).await;
            let student = actor(&pool, &hash, "student@x.edu", Role::Student).await;
            let stranger = actor(&pool, &hash, "stranger@x.edu", Role::Teacher).await;

            let mut tx = pool.begin().await.unwrap();
            let course = create_course(
                &mut tx,
                &owner,
                CourseTitle::parse("Algebra").unwrap(),
                None,
            )
            .await
            .unwrap();
            put_member(
                &mut tx,
                &owner,
                course.id,
                student.user_id(),
                CourseRole::Student,
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();

            let mut conn = pool.acquire().await.unwrap();
            let access = course_access(&mut conn, &owner, course.id).await.unwrap();
            assert_eq!(
                access,
                CourseAccess {
                    role: Some(CourseRole::Teacher),
                    is_owner: true,
                    status: CourseStatus::Draft
                }
            );
            let access = course_access(&mut conn, &admin, course.id).await.unwrap();
            assert_eq!(access.role, None);
            assert!(!access.is_owner);

            // A student cannot see a draft course, a stranger never sees it, an unknown id is absent.
            for who in [&student, &stranger] {
                let outcome = course_access(&mut conn, who, course.id).await;
                assert!(
                    matches!(outcome, Err(TeachingError::CourseNotFound)),
                    "{outcome:?}"
                );
            }
            let outcome = course_access(&mut conn, &owner, Uuid::now_v7()).await;
            assert!(
                matches!(outcome, Err(TeachingError::CourseNotFound)),
                "{outcome:?}"
            );

            let mut tx = pool.begin().await.unwrap();
            publish_course(&mut tx, &owner, course.id).await.unwrap();
            tx.commit().await.unwrap();
            let access = course_access(&mut conn, &student, course.id).await.unwrap();
            assert_eq!(access.role, Some(CourseRole::Student));
            assert_eq!(access.status, CourseStatus::Published);
        })
        .await;
    }

    /// A course with an owner, a student who is a direct member and also in a group, and an admin
    /// who made the group.
    struct Roster {
        pool: sqlx::PgPool,
        owner: Actor,
        course: Uuid,
        group: Uuid,
        student: Uuid,
    }

    async fn roster(pool: sqlx::PgPool) -> Roster {
        let hash = identity::hash_password("irrelevant").await.unwrap();
        let admin = actor(&pool, &hash, "admin@x.edu", Role::Admin).await;
        let owner = actor(&pool, &hash, "owner@x.edu", Role::Teacher).await;
        let student = actor(&pool, &hash, "student@x.edu", Role::Student).await;
        let mut tx = pool.begin().await.unwrap();
        let group = crate::org::seed_group(&mut tx, &admin, "7-A", &[student.user_id()])
            .await
            .unwrap();
        let course = create_course(
            &mut tx,
            &owner,
            CourseTitle::parse("Algebra").unwrap(),
            None,
        )
        .await
        .unwrap();
        put_member(
            &mut tx,
            &owner,
            course.id,
            student.user_id(),
            CourseRole::Student,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        Roster {
            pool,
            owner,
            course: course.id,
            group,
            student: student.user_id(),
        }
    }

    impl Roster {
        async fn on_the_roster(&self) -> bool {
            let mut conn = self.pool.acquire().await.unwrap();
            repo::find_member(&mut conn, self.course, self.student)
                .await
                .unwrap()
                .is_some()
        }

        async fn exclusions(&self) -> Vec<Uuid> {
            let mut conn = self.pool.acquire().await.unwrap();
            repo::list_exclusions(&mut conn, self.course, None, 10)
                .await
                .unwrap()
                .into_iter()
                .map(|exclusion| exclusion.user_id)
                .collect()
        }

        fn remove_in_background(&self) -> tokio::task::JoinHandle<Result<(), TeachingError>> {
            let (pool, owner, course, student) = (
                self.pool.clone(),
                self.owner.clone(),
                self.course,
                self.student,
            );
            tokio::spawn(async move {
                let mut tx = pool.begin().await.unwrap();
                remove_member(&mut tx, &owner, course, student).await?;
                tx.commit().await.map_err(TeachingError::from)
            })
        }
    }

    /// A link is in flight and holds the course lock. The removal of a direct student who is in
    /// that group must wait for it and then see the link: a linked group would put the student
    /// back, so the removal records an exclusion. Without the lock it would not see the link,
    /// record nothing, and the student would come back at once.
    #[tokio::test]
    async fn a_removal_waits_for_a_link_in_flight_and_then_excludes() {
        db_test(async |pool| {
            let r = roster(pool).await;

            let mut first = r.pool.begin().await.unwrap();
            let (_, created) = link_group(&mut first, &r.owner, r.course, r.group, None)
                .await
                .unwrap();
            assert!(created);
            let second = r.remove_in_background();
            wait_until_blocked(&r.pool, &second).await;
            first.commit().await.unwrap();

            second.await.unwrap().unwrap();
            assert!(
                !r.on_the_roster().await,
                "the group does not bring them back"
            );
            assert_eq!(r.exclusions().await, [r.student]);
        })
        .await;
    }

    /// An unlink is in flight. The same removal must wait for it and then see no link, so there
    /// is nothing to exclude them from: the exclusion would only be a stale row.
    #[tokio::test]
    async fn a_removal_waits_for_an_unlink_in_flight_and_records_no_exclusion() {
        db_test(async |pool| {
            let r = roster(pool).await;
            let mut setup = r.pool.begin().await.unwrap();
            let (link, _) = link_group(&mut setup, &r.owner, r.course, r.group, None)
                .await
                .unwrap();
            setup.commit().await.unwrap();

            let mut first = r.pool.begin().await.unwrap();
            unlink_group(&mut first, &r.owner, r.course, link.id)
                .await
                .unwrap();
            let second = r.remove_in_background();
            wait_until_blocked(&r.pool, &second).await;
            first.commit().await.unwrap();

            second.await.unwrap().unwrap();
            assert!(!r.on_the_roster().await);
            assert!(r.exclusions().await.is_empty());
        })
        .await;
    }
}
