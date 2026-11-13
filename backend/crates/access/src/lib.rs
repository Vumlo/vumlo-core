//! The only place that decides who may do what (ADR-0004, ADR-0019). Deny by default.

use chrono::{DateTime, Utc};

mod actor;
mod extract;
mod plugin;
#[cfg(feature = "testing")]
mod test_support;
mod unit;

pub use actor::{Actor, PendingActor};
pub use extract::{ActorSource, Authenticated};
pub use plugin::{
    AuthenticatedPlugin, CourseScope, PluginCaller, PluginRoute, PluginSource, PluginUser, Scope,
    authorize_plugin, authorize_plugin_course, authorize_plugin_route, plugin_course_filter,
    plugin_user,
};
pub use unit::{UnitAbility, UnitGrant, UnitRole, unit_role_allowed};

use common::{ApiError, Role, text_enum};

/// The user's role in the course. The owner is a `Teacher`
/// whose course has `owner_id` set to them; that fact travels in [`CourseAccess::is_owner`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CourseRole {
    Teacher,
    Assistant,
    Student,
}
text_enum!(CourseRole { Teacher => "teacher", Assistant => "assistant", Student => "student" });

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CourseStatus {
    Draft,
    Published,
    Archived,
}
text_enum!(CourseStatus { Draft => "draft", Published => "published", Archived => "archived" });

/// How one actor relates to one course. `teaching` fills it from the database; the rules that
/// read it live only here. An admin has no membership: `role` is `None` for them. `unit_grant` is
/// what the actor's unit roles give them on the unit the course sits in (ADR-0094).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CourseAccess {
    pub role: Option<CourseRole>,
    pub is_owner: bool,
    pub status: CourseStatus,
    pub unit_grant: Option<UnitGrant>,
}

impl CourseAccess {
    /// What this access means for `actor`. An institution `student` is never a course teacher or
    /// an owner (ADR-0004): such a role or ownership is ignored. The rules above stop it from
    /// existing; this makes it harmless if it ever does (ADR-0037).
    fn for_actor(self, actor: &Actor) -> Self {
        if actor.role() != Role::Student {
            return self;
        }
        Self {
            role: self.role.filter(|role| *role != CourseRole::Teacher),
            is_owner: false,
            status: self.status,
            unit_grant: None,
        }
    }

    /// Does a unit grant hold `ability`? Viewing needs none: see [`CourseAccess::has_unit_grant`].
    fn unit(self, ability: UnitAbility) -> bool {
        self.unit_grant.is_some_and(|grant| grant.has(ability))
    }

    fn has_unit_grant(self) -> bool {
        self.unit_grant.is_some()
    }

    /// Owner, teacher or assistant: may see every status and manage students.
    fn is_staff(self) -> bool {
        self.is_owner || matches!(self.role, Some(CourseRole::Teacher | CourseRole::Assistant))
    }

    /// A student of the course: on the roster as a student, whatever their institution role.
    pub fn is_student(self) -> bool {
        self.role == Some(CourseRole::Student)
    }

    /// Owner or teacher: the root actions of ADR-0004 that assistants may not do.
    fn is_owner_or_teacher(self) -> bool {
        self.is_owner || self.role == Some(CourseRole::Teacher)
    }
}

/// What decides whether one module or item is open to students: its status and `publish_at`
/// (ADR-0010). `content` fills it from its rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Release {
    pub published: bool,
    pub publish_at: Option<DateTime<Utc>>,
}

impl Release {
    /// Published, and `publish_at` is not after `at` (a `publish_at` equal to `at` counts).
    fn is_open_at(self, at: DateTime<Utc>) -> bool {
        self.published && self.publish_at.is_none_or(|publish_at| publish_at <= at)
    }
}

/// What an actor wants to do. The resource a variant needs travels inside it, so course-level
/// actions can be added as new variants without touching existing callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Read or change one's own profile, password and sessions; log out.
    ManageOwnAccount,
    /// A permission an optional module declared in its manifest (ADR-0066): `roles` are the
    /// institution roles that hold it. The module and the permission name are for the record.
    Module {
        module: &'static str,
        permission: &'static str,
        roles: &'static [Role],
    },
    /// Archive a course and restore it (ADR-0048). A root action: the owner, a course teacher and
    /// admins (ADR-0004). Ignores the status: it is how a course leaves `archived`.
    ArchiveCourse(CourseAccess),
    /// Write content: create, change, publish, reorder and delete modules and items (ADR-0004).
    /// Staff and admins. [`Action::ManageContent`] is its reading half.
    EditContent(CourseAccess),
    /// Restore an exclusion (ADR-0033). The rule of [`Action::ManageRoster`], which is the reading
    /// half.
    ChangeRoster(CourseAccess),
    /// Link and unlink a group (ADR-0040). The rule of [`Action::LinkGroups`], which is the reading
    /// half.
    ChangeGroupLinks(CourseAccess),
    /// Create, list, change, disable and reset users, institution-wide.
    ManageUsers,
    /// Read the institution settings an admin sees, including the mode and the allowed domains
    /// (ADR-0044). The public profile needs no action: it needs no session.
    ViewInstitutionSettings,
    /// Set the links to the desktop app's installers (ADR-0064). Admins only.
    ManageAppDownloads,
    /// Read the administrator reports: people summary and facts, courses of the institution, term
    /// statistics and the audit log (ADR-0053).
    ViewAdminReports,
    /// Create a course; the creator becomes its owner.
    CreateCourse,
    /// List the courses one belongs to. Which ones is decided per course by `ViewCourse`.
    ViewOwnCourses,
    /// Read a course and its roster. A student sees it once it is published, and still when it is
    /// archived (ADR-0048).
    ViewCourse(CourseAccess),
    /// Move a course from draft to published (ADR-0019).
    PublishCourse(CourseAccess),
    /// Add, change or remove a member who holds (or will hold) `role`. Students can be enrolled in
    /// a draft; an archived course takes no change (ADR-0048).
    ManageMembership {
        access: CourseAccess,
        role: CourseRole,
    },
    DeleteCourse(CourseAccess),
    TransferOwnership(CourseAccess),
    /// View the drafts of a course's content (ADR-0004). Reading only, in every status; writing is
    /// [`Action::EditContent`].
    ManageContent(CourseAccess),
    /// Read one module, sub-module or item (ADR-0010, ADR-0020). Staff and admins see drafts; a
    /// student sees it only if the course and every row on the path to it are open at `at`.
    ///
    /// `module` is the top-level module on the path, `submodule` is set when the target is a
    /// sub-module or lies in one, and `item` is set when the target is an item. `policy` has no
    /// clock: the caller reads the time once and passes it in.
    ViewContent {
        access: CourseAccess,
        module: Release,
        submodule: Option<Release>,
        item: Option<Release>,
        at: DateTime<Utc>,
    },
    /// Read the profiles the document editor enforces (ADR-0025), to build its toolbar.
    ViewEditorProfiles,
    /// Save a draft grade (ADR-0028). Staff and admins; not in an archived course.
    GradeAssignment(CourseAccess),
    /// Copy draft grades to published (ADR-0028). Staff and admins; not in an archived course.
    PublishGrades(CourseAccess),
    /// Read the staff gradebook of a course, with every draft value (ADR-0028).
    ViewGradebook(CourseAccess),
    /// Change how a course turns points into a result: the target, the pass mark, the layout and
    /// the groups (ADR-0089). Staff and admins; not in an archived course.
    ManageGradebook(CourseAccess),
    /// Read one's own published grades: only a student of a published or archived course
    /// (ADR-0028, ADR-0048). Staff and admins use the gradebook instead.
    ViewOwnGrades(CourseAccess),
    /// Read one's own published grades in a course one is no longer on the roster of (ADR-0037).
    /// The caller found published grades of the actor in that course; without any there is
    /// nothing to read.
    ViewFormerGrades {
        has_rows: bool,
    },
    /// Save, discard and submit one's own work (ADR-0035): only a student of a published course
    /// (of an archived one the answer is `Archived`). Whether the course lets the student see the
    /// assignment is `ViewContent`'s business.
    SubmitWork(CourseAccess),
    /// Mark an item of the course done, or take the mark off (ADR-0092): only a student of a
    /// published course (of an archived one the answer is `Archived`). Whether the student may see
    /// the item, and whether a rule has opened it, are not decided here.
    MarkDone(CourseAccess),
    /// Read one's own draft and attempts (ADR-0035): only a student of a published or archived course.
    ViewOwnSubmission(CourseAccess),
    /// Read the submitted attempts of a course's students (ADR-0035). Staff and admins, any
    /// course status. Drafts are never read through this: the queries do not select them.
    ViewSubmissions(CourseAccess),
    /// List the group links and the exclusions (ADR-0033). The rights of the matrix row "Enroll/remove
    /// students". Reading only, in every status; writing is [`Action::ChangeRoster`].
    ManageRoster(CourseAccess),
    /// Read the groups a course can be linked to (ADR-0040). The owner, a course teacher and
    /// admins, not an assistant: a linked group puts its students on the roster, and the roster
    /// shows their names. Reading only; writing is [`Action::ChangeGroupLinks`].
    LinkGroups(CourseAccess),
    /// Change a course's title, term, code and description (ADR-0033, ADR-0048). Not a root action;
    /// not in an archived course.
    EditCourse(CourseAccess),
    /// Create, change and delete terms (ADR-0032).
    ManageTerms,
    /// Read terms: the list a course's term is picked from (ADR-0032).
    ViewTerms,
    /// Create, change and delete groups, subgroups and their members (ADR-0032).
    ManageGroups,
    /// Read groups, subgroups and their members. Admins only until task 015 decides how teachers
    /// pick a group (ADR-0032).
    ViewGroups,
    /// List the units one holds a role in. Which ones is decided by the grants themselves.
    ViewOwnUnits,
    /// Read a unit, its roles and its courses (ADR-0094): an admin, or any grant on it. `None` is
    /// "no grant on that unit".
    ViewUnit(Option<UnitGrant>),
    /// Create a course in a unit, or put a course into one or take it out: an admin, or
    /// `manage_courses` on the unit. Carries no [`CourseAccess`], so an archived course may move.
    ManageUnitCourses(Option<UnitGrant>),
    /// Create, rename, move and delete a unit, and grant or revoke roles on it: an admin, or
    /// `manage_units` on its parent (the caller passes the grant on the parent; a top-level unit
    /// has none).
    ManageUnits(Option<UnitGrant>),
    /// Change what the unit roles may do. Admins only.
    ManageUnitRoleAbilities,
}

impl Action {
    /// The same action with the course access read as it applies to `actor`.
    fn for_actor(self, actor: &Actor) -> Self {
        let mine = |access: CourseAccess| access.for_actor(actor);
        // A grant a student somehow holds is ignored (ADR-0037, ADR-0094).
        let unit = |grant: Option<UnitGrant>| grant.filter(|_| actor.role() != Role::Student);
        match self {
            Self::ViewUnit(grant) => Self::ViewUnit(unit(grant)),
            Self::ManageUnitCourses(grant) => Self::ManageUnitCourses(unit(grant)),
            Self::ManageUnits(grant) => Self::ManageUnits(unit(grant)),
            Self::ArchiveCourse(access) => Self::ArchiveCourse(mine(access)),
            Self::EditContent(access) => Self::EditContent(mine(access)),
            Self::ChangeRoster(access) => Self::ChangeRoster(mine(access)),
            Self::ChangeGroupLinks(access) => Self::ChangeGroupLinks(mine(access)),
            Self::ViewCourse(access) => Self::ViewCourse(mine(access)),
            Self::PublishCourse(access) => Self::PublishCourse(mine(access)),
            Self::ManageMembership { access, role } => Self::ManageMembership {
                access: mine(access),
                role,
            },
            Self::DeleteCourse(access) => Self::DeleteCourse(mine(access)),
            Self::TransferOwnership(access) => Self::TransferOwnership(mine(access)),
            Self::ManageContent(access) => Self::ManageContent(mine(access)),
            Self::ViewContent {
                access,
                module,
                submodule,
                item,
                at,
            } => Self::ViewContent {
                access: mine(access),
                module,
                submodule,
                item,
                at,
            },
            Self::GradeAssignment(access) => Self::GradeAssignment(mine(access)),
            Self::PublishGrades(access) => Self::PublishGrades(mine(access)),
            Self::ViewGradebook(access) => Self::ViewGradebook(mine(access)),
            Self::ManageGradebook(access) => Self::ManageGradebook(mine(access)),
            Self::ViewOwnGrades(access) => Self::ViewOwnGrades(mine(access)),
            Self::SubmitWork(access) => Self::SubmitWork(mine(access)),
            Self::MarkDone(access) => Self::MarkDone(mine(access)),
            Self::ViewOwnSubmission(access) => Self::ViewOwnSubmission(mine(access)),
            Self::ViewSubmissions(access) => Self::ViewSubmissions(mine(access)),
            Self::ManageRoster(access) => Self::ManageRoster(mine(access)),
            Self::LinkGroups(access) => Self::LinkGroups(mine(access)),
            Self::EditCourse(access) => Self::EditCourse(mine(access)),
            Self::ManageOwnAccount
            | Self::Module { .. }
            | Self::ManageUsers
            | Self::ViewInstitutionSettings
            | Self::ManageAppDownloads
            | Self::ViewAdminReports
            | Self::CreateCourse
            | Self::ViewOwnCourses
            | Self::ViewEditorProfiles
            | Self::ViewFormerGrades { .. }
            | Self::ManageTerms
            | Self::ViewTerms
            | Self::ManageGroups
            | Self::ViewGroups
            | Self::ViewOwnUnits
            | Self::ManageUnitRoleAbilities => self,
        }
    }

    /// The course an action writes to, for the actions that a closed (archived) course refuses
    /// (ADR-0048). `None` for reading, and for the root actions that still work on an archived
    /// course: restoring, deleting and handing it over.
    fn written_course(self) -> Option<CourseAccess> {
        match self {
            Self::ManageMembership { access, .. }
            | Self::EditCourse(access)
            | Self::EditContent(access)
            | Self::ChangeRoster(access)
            | Self::ChangeGroupLinks(access)
            | Self::GradeAssignment(access)
            | Self::PublishGrades(access)
            | Self::ManageGradebook(access)
            | Self::SubmitWork(access)
            | Self::MarkDone(access) => Some(access),
            Self::ManageOwnAccount
            | Self::Module { .. }
            | Self::ManageUsers
            | Self::ViewInstitutionSettings
            | Self::ManageAppDownloads
            | Self::ViewAdminReports
            | Self::CreateCourse
            | Self::ViewOwnCourses
            | Self::ViewCourse(_)
            | Self::PublishCourse(_)
            | Self::ArchiveCourse(_)
            | Self::DeleteCourse(_)
            | Self::TransferOwnership(_)
            | Self::ManageContent(_)
            | Self::ViewContent { .. }
            | Self::ViewEditorProfiles
            | Self::ViewGradebook(_)
            | Self::ViewOwnGrades(_)
            | Self::ViewFormerGrades { .. }
            | Self::ViewOwnSubmission(_)
            | Self::ViewSubmissions(_)
            | Self::ManageRoster(_)
            | Self::LinkGroups(_)
            | Self::ManageTerms
            | Self::ViewTerms
            | Self::ManageGroups
            | Self::ViewGroups
            | Self::ViewOwnUnits
            | Self::ViewUnit(_)
            | Self::ManageUnitCourses(_)
            | Self::ManageUnits(_)
            | Self::ManageUnitRoleAbilities => None,
        }
    }
}

impl CourseStatus {
    /// Students see a course in these statuses: while it runs and after it ended (ADR-0048).
    fn is_open_to_students(self) -> bool {
        matches!(self, Self::Published | Self::Archived)
    }
}

/// May `institution` role holders have `course` role (ADR-0004)? Admins hold no course role:
/// they act as implicit owners.
pub fn course_role_allowed(institution: Role, course: CourseRole) -> bool {
    match institution {
        Role::Admin => false,
        Role::Teacher => true,
        Role::Student => matches!(course, CourseRole::Assistant | CourseRole::Student),
    }
}

/// May a user with this institution role be a member of a group? Only students (ADR-0032).
pub fn group_role_allowed(institution: Role) -> bool {
    institution == Role::Student
}

/// True when the actor acts in any course as its implicit owner, without a membership. Such
/// actions are audited.
pub fn implicit_course_owner(actor: &Actor) -> bool {
    actor.role() == Role::Admin
}

/// How an actor reached a course without being on it (ADR-0094).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Admin,
    Unit,
}
text_enum!(Via { Admin => "admin", Unit => "unit" });

/// How the actor reached the course without being its staff, or `None` when they are its owner,
/// teacher or assistant. Every change made this way is audited: the course does not show who made
/// it. A grantee who is a student of the course acts through the grant, so they count as `Unit`.
pub fn implicit_access(actor: &Actor, access: CourseAccess) -> Option<Via> {
    if actor.role() == Role::Admin {
        return Some(Via::Admin);
    }
    let access = access.for_actor(actor);
    (access.has_unit_grant() && !access.is_staff()).then_some(Via::Unit)
}

/// The course-level actions the actor may take, for the web app (ADR-0094).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CourseCan {
    pub view_grades: bool,
    pub edit_content: bool,
    pub grade: bool,
    pub manage_roster: bool,
    pub manage_course: bool,
}

/// What the actor may do in the course, each answered by [`authorize`] as the API would: an
/// archived course reads as `false` for a write.
pub fn course_can(actor: &Actor, access: CourseAccess) -> CourseCan {
    let can = |action| authorize(actor, action).is_ok();
    CourseCan {
        view_grades: can(Action::ViewGradebook(access)),
        edit_content: can(Action::EditContent(access)),
        grade: can(Action::GradeAssignment(access)),
        manage_roster: can(Action::ChangeRoster(access)),
        manage_course: can(Action::EditCourse(access)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AccessError {
    /// The actor may not do this.
    #[error("not allowed")]
    Denied,
    /// The actor may do this in an open course, but the course is archived (ADR-0048).
    #[error("the course is archived")]
    Archived,
    /// The user signed in with a temporary password they have not changed yet (ADR-0017): only
    /// logout, `GET /me` and the change of the password are open to them.
    #[error("the password must be changed first")]
    PasswordChangeRequired,
}

impl From<AccessError> for ApiError {
    fn from(error: AccessError) -> Self {
        match error {
            AccessError::Denied => Self::Forbidden("not allowed"),
            AccessError::Archived => Self::Conflict("the course is archived"),
            AccessError::PasswordChangeRequired => {
                Self::Forbidden("the password must be changed first")
            }
        }
    }
}

/// Allows or denies `action`. The match has no wildcard: a new action does not compile
/// until someone decides who may do it.
pub fn authorize(actor: &Actor, action: Action) -> Result<(), AccessError> {
    let action = action.for_actor(actor);
    let is_admin = actor.role() == Role::Admin;
    let allowed = match action {
        Action::ManageOwnAccount => true,
        Action::Module { roles, .. } => roles.contains(&actor.role()),
        Action::ManageUsers
        | Action::ViewInstitutionSettings
        | Action::ManageAppDownloads
        | Action::ViewAdminReports
        | Action::ManageTerms
        | Action::ManageGroups
        | Action::ViewGroups => is_admin,
        Action::ViewTerms => true,
        Action::CreateCourse => matches!(actor.role(), Role::Admin | Role::Teacher),
        Action::ViewOwnCourses => true,
        Action::ViewEditorProfiles => true,
        Action::ViewOwnUnits => true,
        Action::ViewUnit(grant) => is_admin || grant.is_some(),
        Action::ManageUnitCourses(grant) => {
            is_admin || grant.is_some_and(|grant| grant.has(UnitAbility::ManageCourses))
        }
        Action::ManageUnits(grant) => {
            is_admin || grant.is_some_and(|grant| grant.has(UnitAbility::ManageUnits))
        }
        Action::ManageUnitRoleAbilities => is_admin,
        Action::ViewCourse(access) => {
            is_admin
                || access.is_staff()
                || access.has_unit_grant()
                || (access.role == Some(CourseRole::Student) && access.status.is_open_to_students())
        }
        Action::PublishCourse(access) | Action::ArchiveCourse(access) => {
            is_admin || access.is_owner_or_teacher() || access.unit(UnitAbility::ManageCourses)
        }
        Action::LinkGroups(access) | Action::ChangeGroupLinks(access) => {
            is_admin || access.is_owner_or_teacher() || access.unit(UnitAbility::ManageRoster)
        }
        Action::ManageMembership { access, role } => {
            is_admin
                || access.unit(UnitAbility::ManageRoster)
                || match role {
                    CourseRole::Student => access.is_staff(),
                    CourseRole::Teacher | CourseRole::Assistant => access.is_owner_or_teacher(),
                }
        }
        Action::DeleteCourse(access) | Action::TransferOwnership(access) => {
            is_admin || access.is_owner
        }
        Action::ManageContent(access) => is_admin || access.is_staff() || access.has_unit_grant(),
        Action::EditContent(access) => {
            is_admin || access.is_staff() || access.unit(UnitAbility::EditContent)
        }
        Action::GradeAssignment(access)
        | Action::PublishGrades(access)
        | Action::ManageGradebook(access) => {
            is_admin || access.is_staff() || access.unit(UnitAbility::Grade)
        }
        Action::ViewGradebook(access) | Action::ViewSubmissions(access) => {
            is_admin || access.is_staff() || access.unit(UnitAbility::ViewGrades)
        }
        Action::ManageRoster(access) | Action::ChangeRoster(access) => {
            is_admin || access.is_staff() || access.unit(UnitAbility::ManageRoster)
        }
        Action::EditCourse(access) => {
            is_admin || access.is_staff() || access.unit(UnitAbility::ManageCourses)
        }
        Action::ViewFormerGrades { has_rows } => has_rows,
        Action::ViewOwnGrades(access)
        | Action::SubmitWork(access)
        | Action::MarkDone(access)
        | Action::ViewOwnSubmission(access) => {
            access.role == Some(CourseRole::Student) && access.status.is_open_to_students()
        }
        Action::ViewContent {
            access,
            module,
            submodule,
            item,
            at,
        } => {
            is_admin
                || access.is_staff()
                || access.has_unit_grant()
                || (access.role == Some(CourseRole::Student)
                    && access.status.is_open_to_students()
                    && [Some(module), submodule, item]
                        .into_iter()
                        .flatten()
                        .all(|release| release.is_open_at(at)))
        }
    };
    if !allowed {
        return Err(AccessError::Denied);
    }
    // Decided after the permission, so someone who may not do this is not told about the status.
    if action
        .written_course()
        .is_some_and(|access| access.status == CourseStatus::Archived)
    {
        return Err(AccessError::Archived);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
