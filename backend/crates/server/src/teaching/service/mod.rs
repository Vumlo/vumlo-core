//! Business rules. Every function that acts for a user authorizes first, takes the connection it
//! runs on and never begins or commits a transaction.
//!
//! Course-scoped functions start with [`load`]: a course the actor may not view does not exist
//! as far as they can tell (404), whatever they were trying to do. Only after that does the
//! specific action decide between going ahead and 403.

mod admin;
mod cards;
mod info;

use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

pub use admin::{admin_courses, linked_courses_of_group, user_course_facts};
pub use cards::{course_cards, course_scope, student_courses};
pub use info::{
    archive_course, course_candidates, course_info, course_people, create_from_template,
    delete_course, list_templates, make_template, restore_course, update_course_info,
};

use super::model::{
    Course, CourseGroupLink, CourseKind, CourseTitle, CourseUpdate, Exclusion, MemberSource,
    Membership, ModelError,
};
use super::repo;
use crate::actor::Actor;
use crate::assessment::AssessmentError;
use crate::content::ContentError;
use crate::events::{self, AuditEntry, EventsError};
use crate::gradebook::GradebookError;
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
    /// The new owner of a course is not an active teacher or admin (ADR-0037).
    #[error("a course owner must be an active teacher or admin")]
    OwnerNotAllowed,
    #[error("the course owner cannot be removed")]
    OwnerCannotBeRemoved,
    #[error("an archived course cannot be published")]
    NotPublishable,
    /// The search text of the candidates has fewer than two characters.
    #[error("the search needs at least two characters")]
    QueryTooShort,
    /// A grade has a value in the course, so it cannot be deleted (ADR-0048).
    #[error("the course has grades")]
    HasGrades,
    /// An attempt was submitted in the course, so it cannot be deleted (ADR-0048).
    #[error("the course has submitted work")]
    HasSubmissions,
    #[error(transparent)]
    Content(Box<ContentError>),
    #[error(transparent)]
    Gradebook(Box<GradebookError>),
    #[error(transparent)]
    Assessment(Box<AssessmentError>),
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

/// What a new course is made of: a fresh one, or the copy of another (ADR-0048).
struct NewCourse<'a> {
    kind: CourseKind,
    title: &'a CourseTitle,
    term_id: Option<Uuid>,
    code: &'a str,
    description: &'a str,
    /// The course or template it is copied from.
    from: Option<Uuid>,
}

/// Inserts a draft course owned by the actor. Teachers also join it as its `teacher`; an admin
/// owns it without a membership.
async fn insert_owned_course(
    conn: &mut PgConnection,
    actor: &Actor,
    new: NewCourse<'_>,
) -> Result<Course, TeachingError> {
    policy::authorize(actor, Action::CreateCourse)?;
    // The role in the session may be a moment old. Read it again with the user's row locked, so a
    // demotion cannot run between this check and the insert (ADR-0037).
    let creator = identity::find_user_shared(conn, actor.user_id())
        .await?
        .ok_or(TeachingError::UserNotFound)?;
    if creator.status == UserStatus::Disabled || creator.role == identity::Role::Student {
        return Err(TeachingError::Forbidden(PolicyError::Denied));
    }
    let course = repo::insert_copy(
        conn,
        repo::CourseCopy {
            id: Uuid::now_v7(),
            kind: new.kind,
            title: new.title.as_str(),
            owner_id: actor.user_id(),
            term_id: new.term_id,
            code: new.code,
            description: new.description,
        },
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
            meta: match new.from {
                Some(from) => json!({"from": from}),
                None => json!({}),
            },
        },
    )
    .await?;
    Ok(course)
}

/// Creates a draft course owned by the actor.
pub async fn create_course(
    conn: &mut PgConnection,
    actor: &Actor,
    title: CourseTitle,
    term_id: Option<Uuid>,
) -> Result<Course, TeachingError> {
    insert_owned_course(
        conn,
        actor,
        NewCourse {
            kind: CourseKind::Regular,
            title: &title,
            term_id,
            code: "",
            description: "",
            from: None,
        },
    )
    .await
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
    // The user's row stays locked until the commit: a role change waits for this write and a
    // write that begins after it sees the new role (ADR-0037).
    let user = identity::find_user_shared(conn, user_id)
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

/// Does the user own a course? An owner cannot become a student (ADR-0037).
pub async fn owns_courses(conn: &mut PgConnection, user_id: Uuid) -> Result<bool, TeachingError> {
    Ok(repo::owns_courses(conn, user_id).await?)
}

/// Deletes the user's `teacher` memberships, for the role change that follows in the same
/// transaction, and audits each one (ADR-0037). Acts for the admin who changes the role: the
/// caller has authorized `ManageUsers`.
pub async fn drop_teacher_memberships(
    conn: &mut PgConnection,
    actor: &Actor,
    user_id: Uuid,
) -> Result<(), TeachingError> {
    for course_id in repo::delete_teacher_memberships(conn, user_id).await? {
        events::audit(
            conn,
            actor,
            AuditEntry {
                action: "course.member.remove",
                resource_type: "course",
                resource_id: course_id,
                meta: json!({"user_id": user_id, "role": "teacher", "reason": "role_change"}),
            },
        )
        .await?;
    }
    Ok(())
}

/// Hands the course to another teacher or admin (ADR-0037). The owner or an admin may do it. The
/// new owner becomes a course teacher, unless they are an admin, who own without a membership;
/// the previous owner stays a course teacher. Audited whoever makes it.
pub async fn transfer_ownership(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    new_owner_id: Uuid,
) -> Result<Course, TeachingError> {
    let (course, access) = load(conn, actor, course_id, true).await?;
    policy::authorize(actor, Action::TransferOwnership(access))?;
    if new_owner_id == course.owner_id {
        return Ok(course);
    }
    let user = identity::find_user_shared(conn, new_owner_id)
        .await?
        .ok_or(TeachingError::UserNotFound)?;
    if user.status == UserStatus::Disabled
        || !matches!(user.role, identity::Role::Teacher | identity::Role::Admin)
    {
        return Err(TeachingError::OwnerNotAllowed);
    }
    if user.role == identity::Role::Teacher {
        repo::upsert_membership(conn, course_id, new_owner_id, CourseRole::Teacher).await?;
        repo::delete_exclusion(conn, course_id, new_owner_id).await?;
    }
    let updated = repo::set_owner(conn, course_id, new_owner_id).await?;
    events::audit(
        conn,
        actor,
        AuditEntry {
            action: "course.owner.transfer",
            resource_type: "course",
            resource_id: course_id,
            meta: json!({"from": course.owner_id, "to": new_owner_id}),
        },
    )
    .await?;
    Ok(updated)
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
    policy::authorize(actor, Action::ChangeGroupLinks(access))?;
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
    policy::authorize(actor, Action::ChangeGroupLinks(access))?;
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
    policy::authorize(actor, Action::LinkGroups(access))?;
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
    policy::authorize(actor, Action::ChangeRoster(access))?;
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
mod tests;
