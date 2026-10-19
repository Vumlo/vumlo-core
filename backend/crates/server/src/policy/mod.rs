//! The only place that decides who may do what (ADR-0004, ADR-0019). Deny by default.

use chrono::{DateTime, Utc};

use crate::actor::Actor;
use crate::identity::Role;
use db::text_enum;

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
/// read it live only here. An admin has no membership: `role` is `None` for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CourseAccess {
    pub role: Option<CourseRole>,
    pub is_owner: bool,
    pub status: CourseStatus,
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
        }
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
}

impl Action {
    /// The same action with the course access read as it applies to `actor`.
    fn for_actor(self, actor: &Actor) -> Self {
        let mine = |access: CourseAccess| access.for_actor(actor);
        match self {
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
            Self::ViewOwnGrades(access) => Self::ViewOwnGrades(mine(access)),
            Self::SubmitWork(access) => Self::SubmitWork(mine(access)),
            Self::ViewOwnSubmission(access) => Self::ViewOwnSubmission(mine(access)),
            Self::ViewSubmissions(access) => Self::ViewSubmissions(mine(access)),
            Self::ManageRoster(access) => Self::ManageRoster(mine(access)),
            Self::LinkGroups(access) => Self::LinkGroups(mine(access)),
            Self::EditCourse(access) => Self::EditCourse(mine(access)),
            Self::ManageOwnAccount
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
            | Self::ViewGroups => self,
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
            | Self::SubmitWork(access) => Some(access),
            Self::ManageOwnAccount
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
            | Self::ViewGroups => None,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    /// The actor may not do this.
    #[error("not allowed")]
    Denied,
    /// The actor may do this in an open course, but the course is archived (ADR-0048).
    #[error("the course is archived")]
    Archived,
}

/// Allows or denies `action`. The match has no wildcard: a new action does not compile
/// until someone decides who may do it.
pub fn authorize(actor: &Actor, action: Action) -> Result<(), PolicyError> {
    let action = action.for_actor(actor);
    let is_admin = actor.role() == Role::Admin;
    let allowed = match action {
        Action::ManageOwnAccount => true,
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
        Action::ViewCourse(access) => {
            is_admin
                || access.is_staff()
                || (access.role == Some(CourseRole::Student) && access.status.is_open_to_students())
        }
        Action::PublishCourse(access)
        | Action::ArchiveCourse(access)
        | Action::LinkGroups(access)
        | Action::ChangeGroupLinks(access) => is_admin || access.is_owner_or_teacher(),
        Action::ManageMembership { access, role } => {
            is_admin
                || match role {
                    CourseRole::Student => access.is_staff(),
                    CourseRole::Teacher | CourseRole::Assistant => access.is_owner_or_teacher(),
                }
        }
        Action::DeleteCourse(access) | Action::TransferOwnership(access) => {
            is_admin || access.is_owner
        }
        Action::ManageContent(access)
        | Action::EditContent(access)
        | Action::GradeAssignment(access)
        | Action::PublishGrades(access)
        | Action::ViewGradebook(access)
        | Action::ViewSubmissions(access)
        | Action::ManageRoster(access)
        | Action::ChangeRoster(access)
        | Action::EditCourse(access) => is_admin || access.is_staff(),
        Action::ViewFormerGrades { has_rows } => has_rows,
        Action::ViewOwnGrades(access)
        | Action::SubmitWork(access)
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
                || (access.role == Some(CourseRole::Student)
                    && access.status.is_open_to_students()
                    && [Some(module), submodule, item]
                        .into_iter()
                        .flatten()
                        .all(|release| release.is_open_at(at)))
        }
    };
    if !allowed {
        return Err(PolicyError::Denied);
    }
    // Decided after the permission, so someone who may not do this is not told about the status.
    if action
        .written_course()
        .is_some_and(|access| access.status == CourseStatus::Archived)
    {
        return Err(PolicyError::Archived);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
