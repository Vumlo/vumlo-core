//! Business rules. Every function that acts for a user authorizes first, takes the connection it
//! runs on and never begins or commits a transaction.
//!
//! Course-scoped functions start with [`load`]: a course the actor may not view does not exist
//! as far as they can tell (404), whatever they were trying to do. Only after that does the
//! specific action decide between going ahead and 403.

mod admin;
mod announcements;
mod cards;
mod info;

use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

pub use admin::{
    admin_courses, linked_courses_of_group, regular_courses_per_term, user_course_facts,
};
pub use announcements::{
    delete_announcement, list_announcements, publish_announcement, update_announcement,
};
pub use cards::{course_scope, student_courses};
pub use info::{
    archive_course, begin_delete_course, course_candidates, course_header, course_people,
    finish_delete_course, former_people, list_templates, new_course_from_template, new_template,
    restore_course, update_course_info,
};

use super::model::{
    Course, CourseGroupLink, CourseKind, CourseTitle, CourseUpdate, Exclusion, MemberSource,
    Membership, ModelError, Publication,
};
use super::repo;
use access::Actor;
use access::{
    self, AccessError, Action, CourseAccess, CourseRole, CourseStatus, UnitAbility, UnitGrant,
    course_role_allowed, implicit_course_owner,
};
use auth::{self, AuthError, UserStatus};
use common::{Page, PageParams};
use events::{self, AuditEntry, Event, EventsError};
use org::{self, GroupOption, OrgError};

#[derive(Debug, thiserror::Error)]
pub enum TeachingError {
    #[error(transparent)]
    Forbidden(#[from] AccessError),
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
    /// The `unit_id` of a new course or of a move names no unit.
    #[error("unit not found")]
    UnitNotValid,
    #[error("a template has no term")]
    TemplateHasNoTerm,
    #[error("link not found")]
    LinkNotFound,
    #[error("exclusion not found")]
    ExclusionNotFound,
    /// No such announcement, or its course is not one the actor may see (ADR-0076).
    #[error("announcement not found")]
    AnnouncementNotFound,
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
    #[error(transparent)]
    Identity(#[from] AuthError),
    #[error(transparent)]
    Org(#[from] OrgError),
    #[error(transparent)]
    Events(#[from] EventsError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// How the actor stands to the course. The course cards and the search scope pass no
/// `unit_grant`: they list the courses one is on, and a grant adds none of those.
fn access_of(
    actor: &Actor,
    course: &Course,
    role: Option<CourseRole>,
    unit_grant: Option<UnitGrant>,
) -> CourseAccess {
    CourseAccess {
        role,
        is_owner: course.owner_id == actor.user_id(),
        status: course.status,
        unit_grant,
    }
}

/// The grant the abilities read from `org.unit_grants` make on `unit_id`. A text that is no ability
/// is an error, never a smaller grant: dropping it would silently take a right away (or, were the
/// rule ever inverted, give one).
fn unit_grant_of(unit_id: Uuid, abilities: Vec<String>) -> Result<UnitGrant, TeachingError> {
    let abilities = abilities
        .iter()
        .map(|ability| ability.parse::<UnitAbility>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|message| sqlx::Error::Decode(message.into()))?;
    Ok(UnitGrant::new(unit_id, abilities))
}

/// The actor's grant on the unit the course sits in, from the abilities a query returned.
fn course_unit_grant(
    course: &Course,
    abilities: Option<Vec<String>>,
) -> Result<Option<UnitGrant>, TeachingError> {
    match (course.unit_id, abilities) {
        (Some(unit_id), Some(abilities)) => unit_grant_of(unit_id, abilities).map(Some),
        _ => Ok(None),
    }
}

/// Loads the course as seen by `actor` and refuses with `CourseNotFound` unless they may view it.
/// The actor's role is their place on the roster, the view `teaching.effective_members`: a
/// direct membership, or a student through a linked group (ADR-0033).
///
/// With `lock`, the course row stays locked until the transaction ends, and the actor's role and
/// their grant on the course's unit are read in later statements, after the lock is held. One
/// statement joining them would wait for the lock and then re-read only the course row, leaving
/// the role as of the statement's snapshot: an assistant demoted by the writer we waited for
/// would still act as an assistant, and a manager whose role was revoked would still manage.
/// Without `lock` nothing waits, so one statement is enough.
async fn load(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    lock: bool,
) -> Result<(Course, CourseAccess), TeachingError> {
    let (course, role, abilities) = if lock {
        let course = repo::lock_course(conn, course_id)
            .await?
            .ok_or(TeachingError::CourseNotFound)?;
        let role = repo::find_member(conn, course_id, actor.user_id())
            .await?
            .map(|entry| entry.role);
        let abilities = match course.unit_id {
            Some(unit_id) => repo::find_unit_grant(conn, unit_id, actor.user_id()).await?,
            None => None,
        };
        (course, role, abilities)
    } else {
        let row = repo::find_course(conn, course_id, actor.user_id())
            .await?
            .ok_or(TeachingError::CourseNotFound)?;
        (row.course, row.role, row.unit_abilities)
    };
    let unit_grant = course_unit_grant(&course, abilities)?;
    let access = access_of(actor, &course, role, unit_grant);
    access::authorize(actor, Action::ViewCourse(access))
        .map_err(|_| TeachingError::CourseNotFound)?;
    Ok((course, access))
}

/// An unknown `term_id` violates the foreign key to `org.terms`, a unit deleted since we looked
/// the one to `org.units`.
fn course_fk(error: sqlx::Error) -> TeachingError {
    match &error {
        sqlx::Error::Database(db) if db.is_foreign_key_violation() => match db.constraint() {
            Some("courses_term_id_fkey") => TeachingError::TermNotFound,
            Some("courses_unit_id_fkey") => TeachingError::UnitNotValid,
            _ => TeachingError::Db(error),
        },
        _ => TeachingError::Db(error),
    }
}

/// The actor's grant on a unit named by a request body, which must exist (`UnitNotValid` if not).
async fn body_unit_grant(
    conn: &mut PgConnection,
    actor: &Actor,
    unit_id: Uuid,
) -> Result<Option<UnitGrant>, TeachingError> {
    org::unit_grant(conn, actor, unit_id)
        .await
        .map_err(|error| match error {
            OrgError::UnitNotFound => TeachingError::UnitNotValid,
            other => other.into(),
        })
}

/// May the actor put courses into this unit? No unit needs no grant.
async fn authorize_unit_courses(
    conn: &mut PgConnection,
    actor: &Actor,
    unit_id: Option<Uuid>,
) -> Result<(), TeachingError> {
    if let Some(unit_id) = unit_id {
        let grant = body_unit_grant(conn, actor, unit_id).await?;
        access::authorize(actor, Action::ManageUnitCourses(grant))?;
    }
    Ok(())
}

/// What a new course is made of: a fresh one, or the copy of another (ADR-0048).
struct NewCourse<'a> {
    kind: CourseKind,
    title: &'a CourseTitle,
    term_id: Option<Uuid>,
    /// The unit it is created in. A copy never takes the source's: the creator picks one.
    unit_id: Option<Uuid>,
    code: &'a str,
    description: &'a str,
    /// The course or template it is copied from.
    from: Option<Uuid>,
}

/// Inserts a draft course owned by the actor. Teachers also join it as its `teacher`; an admin
/// owns it without a membership. In a unit it needs `manage_courses` there (or an admin).
async fn insert_owned_course(
    conn: &mut PgConnection,
    actor: &Actor,
    new: NewCourse<'_>,
) -> Result<Course, TeachingError> {
    access::authorize(actor, Action::CreateCourse)?;
    authorize_unit_courses(conn, actor, new.unit_id).await?;
    // The role in the session may be a moment old. Read it again with the user's row locked, so a
    // demotion cannot run between this check and the insert (ADR-0037).
    let creator = auth::find_user_shared(conn, actor.user_id())
        .await?
        .ok_or(TeachingError::UserNotFound)?;
    if creator.status == UserStatus::Disabled || creator.role == common::Role::Student {
        return Err(TeachingError::Forbidden(AccessError::Denied));
    }
    let course = repo::insert_copy(
        conn,
        repo::CourseCopy {
            id: Uuid::now_v7(),
            kind: new.kind,
            title: new.title.as_str(),
            owner_id: actor.user_id(),
            term_id: new.term_id,
            unit_id: new.unit_id,
            code: new.code,
            description: new.description,
        },
    )
    .await
    .map_err(course_fk)?;
    let role = if implicit_course_owner(actor) {
        None
    } else {
        repo::upsert_membership(conn, course.id, actor.user_id(), CourseRole::Teacher).await?;
        Some(CourseRole::Teacher)
    };
    // The creator is the owner, so they are on the course whatever they hold on its unit: the
    // access names no grant and a manager's own new course is not audited as theirs.
    let access = CourseAccess {
        role,
        is_owner: true,
        status: course.status,
        unit_grant: None,
    };
    events::audit_implicit(
        conn,
        actor,
        access,
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

/// Creates a draft course owned by the actor, in `unit_id` when given (it needs `manage_courses`
/// there, or an admin).
pub async fn create_course(
    conn: &mut PgConnection,
    actor: &Actor,
    title: CourseTitle,
    term_id: Option<Uuid>,
    unit_id: Option<Uuid>,
) -> Result<Course, TeachingError> {
    insert_owned_course(
        conn,
        actor,
        NewCourse {
            kind: CourseKind::Regular,
            title: &title,
            term_id,
            unit_id,
            code: "",
            description: "",
            from: None,
        },
    )
    .await
}

/// Whether there is such a course, for a caller that is not a person and so has no view of it to
/// ask for (a plugin, ADR-0072). It reveals nothing but that.
pub async fn course_exists(conn: &mut PgConnection, course_id: Uuid) -> Result<bool, sqlx::Error> {
    repo::course_exists(conn, course_id).await
}

pub async fn get_course(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<Course, TeachingError> {
    Ok(load(conn, actor, course_id, false).await?.0)
}

/// The course and what the actor may do in it, for the course page (ADR-0094).
pub async fn get_course_detail(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<(Course, access::CourseCan), TeachingError> {
    let (course, access) = load(conn, actor, course_id, false).await?;
    Ok((course, access::course_can(actor, access)))
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
    access::authorize(actor, Action::ViewOwnCourses)?;
    let rows =
        repo::list_courses_of(conn, actor.user_id(), page.after, i64::from(page.limit) + 1).await?;
    let mut visible = Vec::with_capacity(rows.len());
    for row in rows {
        let unit_grant = course_unit_grant(&row.course, row.unit_abilities)?;
        let access = access_of(actor, &row.course, row.role, unit_grant);
        if access::authorize(actor, Action::ViewCourse(access)).is_ok() {
            visible.push(row.course);
        }
    }
    Ok(Page::from_rows(visible, page.limit, |course| course.id))
}

/// Puts the course into a unit, takes it out (`None`) or moves it to another. Needs
/// `manage_courses` on the unit it leaves and on the one it enters (an admin has both); a course
/// with no unit has nothing to leave, so a plain teacher cannot put even their own course into a
/// unit they hold nothing on. This is not `EditCourse`, so an archived course moves too.
///
/// The actor must also be staff of the course, an admin or hold a grant on its unit
/// (`ManageContent`): a student of the course (a teacher enrolled as one, say) must not pull it
/// under a unit they manage and so gain the grant's view of it, and a move with no unit on either
/// side still asks that of the caller.
///
/// `load` locks the course first and authorizes viewing it right after, as it does for every
/// writer: someone who may not see the course waits for the row only as long as the writer in
/// front of them holds it, then gets `CourseNotFound`.
pub async fn set_course_unit(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    unit_id: Option<Uuid>,
) -> Result<Course, TeachingError> {
    let (course, access) = load(conn, actor, course_id, true).await?;
    access::authorize(actor, Action::ManageContent(access))?;
    if course.unit_id.is_some() {
        access::authorize(actor, Action::ManageUnitCourses(access.unit_grant))?;
    }
    authorize_unit_courses(conn, actor, unit_id).await?;
    if unit_id == course.unit_id {
        return Ok(course);
    }
    let moved = repo::set_unit(conn, course_id, unit_id)
        .await
        .map_err(course_fk)?;
    events::audit_implicit(
        conn,
        actor,
        access,
        AuditEntry {
            action: "course.unit",
            resource_type: "course",
            resource_id: course_id,
            meta: json!({"from": course.unit_id, "to": unit_id}),
        },
    )
    .await?;
    Ok(moved)
}

/// The courses in a unit and in every unit below it, in any status. A unit the actor holds no
/// grant on does not exist for them (an admin sees all).
pub async fn list_unit_courses(
    conn: &mut PgConnection,
    actor: &Actor,
    unit_id: Uuid,
    page: PageParams,
) -> Result<Page<Course>, TeachingError> {
    let grant = org::unit_grant(conn, actor, unit_id).await?;
    access::authorize(actor, Action::ViewUnit(grant)).map_err(|_| OrgError::UnitNotFound)?;
    let rows =
        repo::list_unit_courses(conn, unit_id, page.after, i64::from(page.limit) + 1).await?;
    Ok(Page::from_rows(rows, page.limit, |course| course.id))
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

/// Moves a draft course to published. Publishing a published course changes nothing, and says
/// so: the flow (`learning::courses::publish`) announces the assignments the course was hiding
/// only when `Publication::changed` is set (ADR-0091).
pub async fn publish_course(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<Publication, TeachingError> {
    let (course, access) = load(conn, actor, course_id, true).await?;
    access::authorize(actor, Action::PublishCourse(access))?;
    match course.status {
        CourseStatus::Published => Ok(Publication {
            course,
            changed: false,
        }),
        CourseStatus::Archived => Err(TeachingError::NotPublishable),
        CourseStatus::Draft => {
            let published = repo::set_published(conn, course_id).await?;
            events::audit_implicit(
                conn,
                actor,
                access,
                AuditEntry {
                    action: "course.publish",
                    resource_type: "course",
                    resource_id: course_id,
                    meta: json!({}),
                },
            )
            .await?;
            Ok(Publication {
                course: published,
                changed: true,
            })
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
    access::authorize(actor, Action::ManageMembership { access, role })?;
    let current = repo::find_member(conn, course_id, user_id).await?;
    if let Some(old) = current {
        access::authorize(
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
    let user = auth::find_user_shared(conn, user_id)
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
    enrollment_changed(conn, course_id, [user_id]).await?;
    events::audit_implicit(
        conn,
        actor,
        access,
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

/// Tells the plugins that the place of these people on the roster of the course may have changed
/// (ADR-0075).
async fn enrollment_changed(
    conn: &mut PgConnection,
    course_id: Uuid,
    users: impl IntoIterator<Item = Uuid>,
) -> Result<(), EventsError> {
    for user_id in users {
        events::emit(conn, Event::EnrollmentChanged { course_id, user_id }).await?;
    }
    Ok(())
}

/// Forgets what was deleted more than `days` ago (ADR-0081). Authorizes nothing: the delivery worker
/// of the plugins calls it, and nobody acts through it.
pub async fn prune_tombstones(conn: &mut PgConnection, days: i32) -> Result<u64, TeachingError> {
    Ok(repo::prune_tombstones(conn, days).await?)
}

/// Does the user own a course? An owner cannot become a student (ADR-0037).
pub async fn owns_courses(conn: &mut PgConnection, user_id: Uuid) -> Result<bool, TeachingError> {
    Ok(repo::owns_courses(conn, user_id).await?)
}

/// Deletes the user's `teacher` memberships, for the role change that follows in the same
/// transaction, and audits each one (ADR-0037). Acts for the admin who changes the role: the
/// caller has authorized `ManageUsers`. Returns the courses they were in, for
/// [`user_standing_changed`].
pub async fn drop_teacher_memberships(
    conn: &mut PgConnection,
    actor: &Actor,
    user_id: Uuid,
) -> Result<Vec<Uuid>, TeachingError> {
    let courses = repo::delete_teacher_memberships(conn, user_id).await?;
    for &course_id in &courses {
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
    Ok(courses)
}

/// Tells the plugins that a user's role or status changed, in every course whose roster they are
/// on, or were on before this change (`left`: courses the change took them off, which no query
/// can find any more). A disabled student leaves every roster and a role change moves a person
/// between the roster of a group and that of a membership (ADR-0075).
pub async fn user_standing_changed(
    conn: &mut PgConnection,
    user_id: Uuid,
    left: Vec<Uuid>,
) -> Result<(), TeachingError> {
    let mut courses: std::collections::BTreeSet<Uuid> = left.into_iter().collect();
    courses.extend(repo::courses_with_user(conn, user_id).await?);
    for course_id in courses {
        enrollment_changed(conn, course_id, [user_id]).await?;
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
    access::authorize(actor, Action::TransferOwnership(access))?;
    if new_owner_id == course.owner_id {
        return Ok(course);
    }
    let user = auth::find_user_shared(conn, new_owner_id)
        .await?
        .ok_or(TeachingError::UserNotFound)?;
    if user.status == UserStatus::Disabled
        || !matches!(user.role, common::Role::Teacher | common::Role::Admin)
    {
        return Err(TeachingError::OwnerNotAllowed);
    }
    if user.role == common::Role::Teacher {
        repo::upsert_membership(conn, course_id, new_owner_id, CourseRole::Teacher).await?;
        repo::delete_exclusion(conn, course_id, new_owner_id).await?;
        enrollment_changed(conn, course_id, [new_owner_id]).await?;
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
    access::authorize(
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
    enrollment_changed(conn, course_id, [user_id]).await?;
    events::audit_implicit(
        conn,
        actor,
        access,
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
    access::authorize(actor, Action::EditCourse(access))?;
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
    .map_err(course_fk)?;
    events::audit_implicit(
        conn,
        actor,
        access,
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
    access::authorize(actor, Action::ManageRoster(access))?;
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
    access::authorize(actor, Action::ChangeGroupLinks(access))?;
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
    let brought = repo::link_users(conn, link.id).await?;
    enrollment_changed(conn, course_id, brought).await?;
    events::audit_implicit(
        conn,
        actor,
        access,
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
    access::authorize(actor, Action::ChangeGroupLinks(access))?;
    // Read before the link goes: the view of its students is empty after.
    let leaving = repo::link_users(conn, link_id).await?;
    let link = repo::delete_link(conn, course_id, link_id)
        .await?
        .ok_or(TeachingError::LinkNotFound)?;
    enrollment_changed(conn, course_id, leaving).await?;
    events::audit_implicit(
        conn,
        actor,
        access,
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
    access::authorize(actor, Action::LinkGroups(access))?;
    Ok(org::group_options(conn, page).await?)
}

pub async fn list_exclusions(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    page: PageParams,
) -> Result<Page<Exclusion>, TeachingError> {
    let (_, access) = load(conn, actor, course_id, false).await?;
    access::authorize(actor, Action::ManageRoster(access))?;
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
    access::authorize(actor, Action::ChangeRoster(access))?;
    if repo::delete_exclusion(conn, course_id, user_id).await? == 0 {
        return Err(TeachingError::ExclusionNotFound);
    }
    enrollment_changed(conn, course_id, [user_id]).await?;
    events::audit_implicit(
        conn,
        actor,
        access,
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
