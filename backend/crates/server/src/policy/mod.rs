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
mod tests {
    use uuid::Uuid;

    use super::*;

    fn actor(role: Role) -> Actor {
        Actor::new(Uuid::now_v7(), role, Uuid::now_v7())
    }

    #[test]
    fn every_role_may_manage_their_own_account() {
        for role in [Role::Admin, Role::Teacher, Role::Student] {
            assert_eq!(authorize(&actor(role), Action::ManageOwnAccount), Ok(()));
        }
    }

    #[test]
    fn every_role_may_view_the_editor_profiles() {
        for role in [Role::Admin, Role::Teacher, Role::Student] {
            assert_eq!(authorize(&actor(role), Action::ViewEditorProfiles), Ok(()));
        }
    }

    #[test]
    fn only_admins_manage_users() {
        assert_eq!(authorize(&actor(Role::Admin), Action::ManageUsers), Ok(()));
        for role in [Role::Teacher, Role::Student] {
            assert_eq!(
                authorize(&actor(role), Action::ManageUsers),
                Err(PolicyError::Denied),
                "{role:?}"
            );
        }
    }

    #[test]
    fn only_admins_manage_terms_and_groups_and_read_groups() {
        for action in [
            Action::ViewInstitutionSettings,
            Action::ViewAdminReports,
            Action::ManageTerms,
            Action::ManageGroups,
            Action::ViewGroups,
        ] {
            assert_eq!(authorize(&actor(Role::Admin), action), Ok(()), "{action:?}");
            for role in [Role::Teacher, Role::Student] {
                assert_eq!(
                    authorize(&actor(role), action),
                    Err(PolicyError::Denied),
                    "{action:?} {role:?}"
                );
            }
        }
    }

    /// What a write action that staff may do answers in `status` (ADR-0048).
    fn written_in(status: CourseStatus) -> Result<(), PolicyError> {
        if status == Archived {
            Err(PolicyError::Archived)
        } else {
            Ok(())
        }
    }

    #[test]
    fn staff_and_admins_read_the_roster_and_content_in_every_status() {
        let reads: [fn(CourseAccess) -> Action; 2] = [Action::ManageRoster, Action::ManageContent];
        for make in reads {
            for status in [Draft, Published, Archived] {
                for (role, is_owner) in [
                    (Some(Teacher), true),
                    (Some(Teacher), false),
                    (Some(Assistant), false),
                ] {
                    let action = make(access(role, is_owner, status));
                    assert_eq!(
                        authorize(&actor(Role::Teacher), action),
                        Ok(()),
                        "{action:?}"
                    );
                }
                let admin = make(access(None, false, status));
                assert_eq!(authorize(&actor(Role::Admin), admin), Ok(()), "{admin:?}");
                for role in [Some(Student), None] {
                    let action = make(access(role, false, status));
                    for who in [Role::Student, Role::Teacher] {
                        assert_eq!(
                            authorize(&actor(who), action),
                            Err(PolicyError::Denied),
                            "{action:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn staff_and_admins_change_the_roster_content_and_settings_but_not_in_an_archived_course() {
        let writes: [fn(CourseAccess) -> Action; 3] = [
            Action::ChangeRoster,
            Action::EditCourse,
            Action::EditContent,
        ];
        for make in writes {
            for status in [Draft, Published, Archived] {
                for (role, is_owner) in [
                    (Some(Teacher), true),
                    (Some(Teacher), false),
                    (Some(Assistant), false),
                ] {
                    let action = make(access(role, is_owner, status));
                    assert_eq!(
                        authorize(&actor(Role::Teacher), action),
                        written_in(status),
                        "{action:?}"
                    );
                }
                let admin = make(access(None, false, status));
                assert_eq!(
                    authorize(&actor(Role::Admin), admin),
                    written_in(status),
                    "{admin:?}"
                );
                // Someone who may not do it is told so, whatever the status.
                for role in [Some(Student), None] {
                    let action = make(access(role, false, status));
                    for who in [Role::Student, Role::Teacher] {
                        assert_eq!(
                            authorize(&actor(who), action),
                            Err(PolicyError::Denied),
                            "{action:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn archiving_is_a_root_action_in_every_status() {
        for status in [Draft, Published, Archived] {
            for (role, is_owner, who) in [
                (Some(Teacher), true, Role::Teacher),
                (Some(Teacher), false, Role::Teacher),
                (None, false, Role::Admin),
            ] {
                let action = Action::ArchiveCourse(access(role, is_owner, status));
                assert_eq!(authorize(&actor(who), action), Ok(()), "{action:?}");
            }
            for (role, who) in [
                (Some(Assistant), Role::Student),
                (Some(Assistant), Role::Teacher),
                (Some(Student), Role::Student),
                (None, Role::Teacher),
            ] {
                let action = Action::ArchiveCourse(access(role, false, status));
                assert_eq!(
                    authorize(&actor(who), action),
                    Err(PolicyError::Denied),
                    "{action:?}"
                );
            }
        }
    }

    #[test]
    fn restoring_deleting_and_handing_over_still_work_on_an_archived_course() {
        let archived_owner = access(Some(Teacher), true, Archived);
        for action in [
            Action::ArchiveCourse(archived_owner),
            Action::DeleteCourse(archived_owner),
            Action::TransferOwnership(archived_owner),
        ] {
            assert_eq!(
                authorize(&actor(Role::Teacher), action),
                Ok(()),
                "{action:?}"
            );
        }
    }

    #[test]
    fn a_student_submitting_in_an_archived_course_is_told_it_is_archived_not_refused() {
        let student = actor(Role::Student);
        let archived = access(Some(Student), false, Archived);
        assert_eq!(
            authorize(&student, Action::SubmitWork(archived)),
            Err(PolicyError::Archived)
        );
        // Reading their own work and grades there is fine.
        for read in [
            Action::ViewOwnSubmission(archived),
            Action::ViewOwnGrades(archived),
        ] {
            assert_eq!(authorize(&student, read), Ok(()), "{read:?}");
        }
        // A non-student gets the usual refusal, not the status.
        let outsider = access(None, false, Archived);
        assert_eq!(
            authorize(&actor(Role::Teacher), Action::SubmitWork(outsider)),
            Err(PolicyError::Denied)
        );
    }

    #[test]
    fn only_the_owner_a_teacher_and_admins_link_groups() {
        for status in [Draft, Published, Archived] {
            for (role, is_owner) in [(Some(Teacher), true), (Some(Teacher), false)] {
                let action = Action::LinkGroups(access(role, is_owner, status));
                assert_eq!(
                    authorize(&actor(Role::Teacher), action),
                    Ok(()),
                    "{action:?}"
                );
            }
            let admin = Action::LinkGroups(access(None, false, status));
            assert_eq!(authorize(&actor(Role::Admin), admin), Ok(()));
            // An assistant is staff and manages the roster, but does not link groups.
            for (role, who) in [
                (Some(Assistant), Role::Student),
                (Some(Assistant), Role::Teacher),
                (Some(Student), Role::Student),
                (None, Role::Teacher),
            ] {
                let action = Action::LinkGroups(access(role, false, status));
                assert_eq!(
                    authorize(&actor(who), action),
                    Err(PolicyError::Denied),
                    "{action:?}"
                );
            }
        }
    }

    #[test]
    fn every_role_may_read_terms() {
        for role in [Role::Admin, Role::Teacher, Role::Student] {
            assert_eq!(
                authorize(&actor(role), Action::ViewTerms),
                Ok(()),
                "{role:?}"
            );
        }
    }

    #[test]
    fn only_students_are_group_members() {
        assert!(group_role_allowed(Role::Student));
        assert!(!group_role_allowed(Role::Teacher));
        assert!(!group_role_allowed(Role::Admin));
    }

    use CourseRole::{Assistant, Student, Teacher};
    use CourseStatus::{Archived, Draft, Published};

    fn access(role: Option<CourseRole>, is_owner: bool, status: CourseStatus) -> CourseAccess {
        CourseAccess {
            role,
            is_owner,
            status,
        }
    }

    fn member(role: CourseRole) -> CourseAccess {
        access(Some(role), false, Published)
    }

    fn owner() -> CourseAccess {
        access(Some(Teacher), true, Published)
    }

    fn non_member() -> CourseAccess {
        access(None, false, Published)
    }

    #[test]
    fn only_teachers_and_admins_create_courses() {
        assert_eq!(
            authorize(&actor(Role::Teacher), Action::CreateCourse),
            Ok(())
        );
        assert_eq!(authorize(&actor(Role::Admin), Action::CreateCourse), Ok(()));
        assert_eq!(
            authorize(&actor(Role::Student), Action::CreateCourse),
            Err(PolicyError::Denied)
        );
    }

    #[test]
    fn every_role_may_list_their_courses() {
        for role in [Role::Admin, Role::Teacher, Role::Student] {
            assert_eq!(authorize(&actor(role), Action::ViewOwnCourses), Ok(()));
        }
    }

    #[test]
    fn student_member_cannot_add_members_with_any_role() {
        for role in [Teacher, Assistant, Student] {
            let action = Action::ManageMembership {
                access: member(Student),
                role,
            };
            assert_eq!(
                authorize(&actor(Role::Student), action),
                Err(PolicyError::Denied),
                "{role:?}"
            );
        }
    }

    #[test]
    fn assistant_cannot_delete_or_transfer_ownership() {
        let assistant = actor(Role::Student);
        let access = member(Assistant);
        assert_eq!(
            authorize(&assistant, Action::DeleteCourse(access)),
            Err(PolicyError::Denied)
        );
        assert_eq!(
            authorize(&assistant, Action::TransferOwnership(access)),
            Err(PolicyError::Denied)
        );
    }

    #[test]
    fn only_owner_and_admin_delete_or_transfer() {
        let teacher = actor(Role::Teacher);
        for action in [
            Action::DeleteCourse(owner()),
            Action::TransferOwnership(owner()),
        ] {
            assert_eq!(authorize(&teacher, action), Ok(()));
        }
        for action in [
            Action::DeleteCourse(member(Teacher)),
            Action::TransferOwnership(member(Teacher)),
        ] {
            assert_eq!(authorize(&teacher, action), Err(PolicyError::Denied));
        }
        for action in [
            Action::DeleteCourse(non_member()),
            Action::TransferOwnership(non_member()),
        ] {
            assert_eq!(authorize(&actor(Role::Admin), action), Ok(()));
        }
    }

    #[test]
    fn non_member_cannot_view_a_course_but_an_admin_can() {
        for role in [Role::Teacher, Role::Student] {
            for status in [Draft, Published, Archived] {
                assert_eq!(
                    authorize(
                        &actor(role),
                        Action::ViewCourse(access(None, false, status))
                    ),
                    Err(PolicyError::Denied),
                    "{role:?} {status:?}"
                );
            }
        }
        for status in [Draft, Published, Archived] {
            assert_eq!(
                authorize(
                    &actor(Role::Admin),
                    Action::ViewCourse(access(None, false, status))
                ),
                Ok(())
            );
        }
    }

    #[test]
    fn student_member_sees_published_and_archived_courses_but_not_drafts() {
        let student = actor(Role::Student);
        let view = |status| Action::ViewCourse(access(Some(Student), false, status));
        assert_eq!(authorize(&student, view(Published)), Ok(()));
        assert_eq!(authorize(&student, view(Archived)), Ok(()));
        assert_eq!(authorize(&student, view(Draft)), Err(PolicyError::Denied));
    }

    #[test]
    fn staff_see_every_status() {
        for (role, is_owner) in [
            (Some(Teacher), true),
            (Some(Teacher), false),
            (Some(Assistant), false),
        ] {
            for status in [Draft, Published, Archived] {
                assert_eq!(
                    authorize(
                        &actor(Role::Teacher),
                        Action::ViewCourse(access(role, is_owner, status))
                    ),
                    Ok(()),
                    "{role:?} {status:?}"
                );
            }
        }
    }

    #[test]
    fn publishing_is_a_root_action() {
        let publish = |access| Action::PublishCourse(access);
        let draft = |role, is_owner| access(role, is_owner, Draft);
        assert_eq!(
            authorize(&actor(Role::Teacher), publish(draft(Some(Teacher), true))),
            Ok(())
        );
        assert_eq!(
            authorize(&actor(Role::Teacher), publish(draft(Some(Teacher), false))),
            Ok(())
        );
        assert_eq!(
            authorize(&actor(Role::Admin), publish(draft(None, false))),
            Ok(())
        );
        for role in [Assistant, Student] {
            assert_eq!(
                authorize(&actor(Role::Student), publish(draft(Some(role), false))),
                Err(PolicyError::Denied),
                "{role:?}"
            );
        }
        assert_eq!(
            authorize(&actor(Role::Teacher), publish(draft(None, false))),
            Err(PolicyError::Denied)
        );
    }

    #[test]
    fn students_can_be_enrolled_in_a_draft_but_an_archived_course_takes_no_change() {
        for status in [Draft, Published, Archived] {
            let action = Action::ManageMembership {
                access: access(Some(Teacher), false, status),
                role: Student,
            };
            assert_eq!(
                authorize(&actor(Role::Teacher), action),
                written_in(status),
                "{status:?}"
            );
        }
    }

    #[test]
    fn linking_a_group_follows_the_rule_of_reading_the_options_and_stops_at_an_archived_course() {
        for status in [Draft, Published, Archived] {
            for (role, is_owner, who) in [
                (Some(Teacher), true, Role::Teacher),
                (Some(Teacher), false, Role::Teacher),
                (None, false, Role::Admin),
            ] {
                let action = Action::ChangeGroupLinks(access(role, is_owner, status));
                assert_eq!(
                    authorize(&actor(who), action),
                    written_in(status),
                    "{action:?}"
                );
            }
            for (role, who) in [
                (Some(Assistant), Role::Student),
                (Some(Assistant), Role::Teacher),
                (Some(Student), Role::Student),
                (None, Role::Teacher),
            ] {
                let action = Action::ChangeGroupLinks(access(role, false, status));
                assert_eq!(
                    authorize(&actor(who), action),
                    Err(PolicyError::Denied),
                    "{action:?}"
                );
            }
        }
    }

    #[test]
    fn assistant_manages_students_but_not_staff() {
        let assistant = actor(Role::Student);
        let manage = |role| Action::ManageMembership {
            access: member(Assistant),
            role,
        };
        assert_eq!(authorize(&assistant, manage(Student)), Ok(()));
        assert_eq!(
            authorize(&assistant, manage(Teacher)),
            Err(PolicyError::Denied)
        );
        assert_eq!(
            authorize(&assistant, manage(Assistant)),
            Err(PolicyError::Denied)
        );
    }

    #[test]
    fn teacher_and_owner_manage_staff_and_students() {
        for access in [member(Teacher), owner()] {
            for role in [Teacher, Assistant, Student] {
                let action = Action::ManageMembership { access, role };
                assert_eq!(authorize(&actor(Role::Teacher), action), Ok(()), "{role:?}");
            }
        }
    }

    #[test]
    fn non_members_cannot_manage_members_but_admins_can() {
        for role in [Teacher, Assistant, Student] {
            let action = Action::ManageMembership {
                access: non_member(),
                role,
            };
            assert_eq!(
                authorize(&actor(Role::Teacher), action),
                Err(PolicyError::Denied)
            );
            assert_eq!(authorize(&actor(Role::Admin), action), Ok(()));
        }
    }

    #[test]
    fn course_roles_follow_the_institution_role() {
        for role in [Teacher, Assistant, Student] {
            assert!(!course_role_allowed(Role::Admin, role), "admin {role:?}");
            assert!(course_role_allowed(Role::Teacher, role), "teacher {role:?}");
        }
        assert!(!course_role_allowed(Role::Student, Teacher));
        assert!(course_role_allowed(Role::Student, Assistant));
        assert!(course_role_allowed(Role::Student, Student));
    }

    #[test]
    fn only_admins_act_as_implicit_owners() {
        assert!(implicit_course_owner(&actor(Role::Admin)));
        assert!(!implicit_course_owner(&actor(Role::Teacher)));
        assert!(!implicit_course_owner(&actor(Role::Student)));
    }

    fn at() -> DateTime<Utc> {
        "2030-06-01T12:00:00Z".parse().unwrap()
    }

    fn open() -> Release {
        Release {
            published: true,
            publish_at: None,
        }
    }

    fn draft() -> Release {
        Release {
            published: false,
            publish_at: None,
        }
    }

    fn publish_at(offset_secs: i64) -> Release {
        Release {
            published: true,
            publish_at: Some(at() + chrono::Duration::seconds(offset_secs)),
        }
    }

    fn view(
        access: CourseAccess,
        module: Release,
        submodule: Option<Release>,
        item: Option<Release>,
    ) -> Action {
        Action::ViewContent {
            access,
            module,
            submodule,
            item,
            at: at(),
        }
    }

    #[test]
    fn a_student_sees_content_only_when_the_course_and_every_row_on_the_path_are_open() {
        let student = actor(Role::Student);
        let allowed = |action| authorize(&student, action) == Ok(());
        let me = member(Student);
        assert!(allowed(view(me, open(), Some(open()), Some(open()))));
        assert!(allowed(view(me, open(), None, Some(open()))));
        assert!(allowed(view(me, open(), None, None)));
        assert!(allowed(view(me, open(), Some(open()), None)));

        let closed = access(Some(Student), false, Draft);
        assert!(
            !allowed(view(closed, open(), Some(open()), Some(open()))),
            "draft course"
        );
        // An archived course stays readable (ADR-0048), with the same rules for its rows.
        let archived = access(Some(Student), false, Archived);
        assert!(allowed(view(archived, open(), Some(open()), Some(open()))));
        assert!(!allowed(view(
            archived,
            draft(),
            Some(open()),
            Some(open())
        )));
        assert!(
            !allowed(view(me, draft(), Some(open()), Some(open()))),
            "draft module"
        );
        assert!(
            !allowed(view(me, open(), Some(draft()), Some(open()))),
            "draft sub-module"
        );
        assert!(
            !allowed(view(me, open(), Some(open()), Some(draft()))),
            "draft item"
        );
        assert!(
            !allowed(view(me, open(), None, Some(draft()))),
            "draft direct item"
        );
        assert!(
            !allowed(view(me, publish_at(1), Some(open()), Some(open()))),
            "future module"
        );
        assert!(
            !allowed(view(me, open(), Some(publish_at(1)), Some(open()))),
            "future sub-module"
        );
        assert!(
            !allowed(view(me, open(), Some(open()), Some(publish_at(1)))),
            "future item"
        );
        assert!(
            !allowed(view(me, open(), None, Some(publish_at(1)))),
            "future direct item"
        );
    }

    #[test]
    fn a_publish_at_equal_to_the_evaluation_time_is_open_on_every_level() {
        let student = actor(Role::Student);
        let me = member(Student);
        for action in [
            view(me, publish_at(0), Some(open()), Some(open())),
            view(me, open(), Some(publish_at(0)), Some(open())),
            view(me, open(), Some(open()), Some(publish_at(0))),
            view(
                me,
                publish_at(-1),
                Some(publish_at(-1)),
                Some(publish_at(-1)),
            ),
        ] {
            assert_eq!(authorize(&student, action), Ok(()), "{action:?}");
        }
    }

    #[test]
    fn staff_and_admins_see_drafts_and_future_content_in_every_course_status() {
        let hidden = |item| view(item, draft(), Some(publish_at(60)), Some(draft()));
        for status in [Draft, Published, Archived] {
            for (role, is_owner) in [
                (Some(Teacher), true),
                (Some(Teacher), false),
                (Some(Assistant), false),
            ] {
                let action = hidden(access(role, is_owner, status));
                assert_eq!(
                    authorize(&actor(Role::Teacher), action),
                    Ok(()),
                    "{role:?} {status:?}"
                );
            }
            let action = hidden(access(None, false, status));
            assert_eq!(
                authorize(&actor(Role::Admin), action),
                Ok(()),
                "admin {status:?}"
            );
        }
    }

    #[test]
    fn non_members_see_no_content_but_admins_do() {
        let action = view(non_member(), open(), None, Some(open()));
        for role in [Role::Teacher, Role::Student] {
            assert_eq!(
                authorize(&actor(role), action),
                Err(PolicyError::Denied),
                "{role:?}"
            );
        }
        assert_eq!(authorize(&actor(Role::Admin), action), Ok(()));
    }

    #[test]
    fn staff_and_admins_read_the_gradebook_in_every_status_and_grade_and_publish_but_not_in_an_archived_one()
     {
        type Make = fn(CourseAccess) -> Action;
        let actions: [(Make, bool); 3] = [
            (Action::GradeAssignment, true),
            (Action::PublishGrades, true),
            (Action::ViewGradebook, false),
        ];
        for (make, is_write) in actions {
            let expected = |status| {
                if is_write { written_in(status) } else { Ok(()) }
            };
            for status in [Draft, Published, Archived] {
                for (role, is_owner) in [
                    (Some(Teacher), true),
                    (Some(Teacher), false),
                    (Some(Assistant), false),
                ] {
                    let action = make(access(role, is_owner, status));
                    assert_eq!(
                        authorize(&actor(Role::Teacher), action),
                        expected(status),
                        "{action:?}"
                    );
                }
                let admin = make(access(None, false, status));
                assert_eq!(
                    authorize(&actor(Role::Admin), admin),
                    expected(status),
                    "{admin:?}"
                );
                for role in [Some(Student), None] {
                    let action = make(access(role, false, status));
                    for who in [Role::Student, Role::Teacher] {
                        assert_eq!(
                            authorize(&actor(who), action),
                            Err(PolicyError::Denied),
                            "{action:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn only_a_student_of_a_published_course_saves_submits_and_reads_their_own_work() {
        let actions: [fn(CourseAccess) -> Action; 2] =
            [Action::SubmitWork, Action::ViewOwnSubmission];
        for make in actions {
            assert_eq!(
                authorize(
                    &actor(Role::Student),
                    make(access(Some(Student), false, Published))
                ),
                Ok(())
            );
            assert_eq!(
                authorize(
                    &actor(Role::Student),
                    make(access(Some(Student), false, Draft))
                ),
                Err(PolicyError::Denied),
                "draft"
            );
            // Staff, a non-member and an admin (who has no membership) never submit.
            for (role, is_owner) in [
                (Some(Teacher), true),
                (Some(Teacher), false),
                (Some(Assistant), false),
                (None, false),
            ] {
                for who in [Role::Teacher, Role::Student] {
                    assert_eq!(
                        authorize(&actor(who), make(access(role, is_owner, Published))),
                        Err(PolicyError::Denied),
                        "{role:?} {who:?}"
                    );
                }
            }
            assert_eq!(
                authorize(&actor(Role::Admin), make(access(None, false, Published))),
                Err(PolicyError::Denied)
            );
        }
    }

    #[test]
    fn staff_and_admins_read_submissions_in_every_status_and_students_never_do() {
        for status in [Draft, Published, Archived] {
            for (role, is_owner) in [
                (Some(Teacher), true),
                (Some(Teacher), false),
                (Some(Assistant), false),
            ] {
                let action = Action::ViewSubmissions(access(role, is_owner, status));
                assert_eq!(
                    authorize(&actor(Role::Teacher), action),
                    Ok(()),
                    "{action:?}"
                );
            }
            let admin = Action::ViewSubmissions(access(None, false, status));
            assert_eq!(authorize(&actor(Role::Admin), admin), Ok(()));
            for role in [Some(Student), None] {
                let action = Action::ViewSubmissions(access(role, false, status));
                for who in [Role::Student, Role::Teacher] {
                    assert_eq!(
                        authorize(&actor(who), action),
                        Err(PolicyError::Denied),
                        "{action:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn only_a_student_of_a_published_course_views_their_own_grades() {
        let action = |role, is_owner, status| Action::ViewOwnGrades(access(role, is_owner, status));
        assert_eq!(
            authorize(
                &actor(Role::Student),
                action(Some(Student), false, Published)
            ),
            Ok(())
        );
        assert_eq!(
            authorize(&actor(Role::Student), action(Some(Student), false, Draft)),
            Err(PolicyError::Denied),
            "draft"
        );
        // After the course ended the student still reads their grades (ADR-0048).
        assert_eq!(
            authorize(
                &actor(Role::Student),
                action(Some(Student), false, Archived)
            ),
            Ok(())
        );
        for (role, is_owner) in [
            (Some(Teacher), true),
            (Some(Teacher), false),
            (Some(Assistant), false),
            (None, false),
        ] {
            assert_eq!(
                authorize(&actor(Role::Teacher), action(role, is_owner, Published)),
                Err(PolicyError::Denied),
                "{role:?}"
            );
        }
        // An admin has no membership and is not a student.
        assert_eq!(
            authorize(&actor(Role::Admin), action(None, false, Published)),
            Err(PolicyError::Denied)
        );
    }

    #[test]
    fn staff_and_admins_view_content_drafts_in_every_course_status_and_students_never_do() {
        for status in [Draft, Published, Archived] {
            for (role, is_owner) in [
                (Some(Teacher), true),
                (Some(Teacher), false),
                (Some(Assistant), false),
            ] {
                let action = Action::ManageContent(access(role, is_owner, status));
                assert_eq!(
                    authorize(&actor(Role::Teacher), action),
                    Ok(()),
                    "{role:?} {status:?}"
                );
            }
            assert_eq!(
                authorize(
                    &actor(Role::Admin),
                    Action::ManageContent(access(None, false, status))
                ),
                Ok(())
            );
            for role in [Some(Student), None] {
                let action = Action::ManageContent(access(role, false, status));
                assert_eq!(
                    authorize(&actor(Role::Student), action),
                    Err(PolicyError::Denied),
                    "{role:?} {status:?}"
                );
                assert_eq!(
                    authorize(&actor(Role::Teacher), action),
                    Err(PolicyError::Denied),
                    "{role:?} {status:?}"
                );
            }
        }
    }

    #[test]
    fn course_enums_round_trip() {
        for role in [Teacher, Assistant, Student] {
            assert_eq!(role.as_str().parse::<CourseRole>(), Ok(role));
        }
        for status in [Draft, Published, Archived] {
            assert_eq!(status.as_str().parse::<CourseStatus>(), Ok(status));
        }
        assert!("owner".parse::<CourseRole>().is_err());
    }

    #[test]
    fn an_institution_student_is_never_a_course_teacher_or_owner() {
        // A teacher membership or ownership held by an institution student is ignored (ADR-0037).
        let student = actor(Role::Student);
        let stale = |role, is_owner| access(Some(role), is_owner, Published);
        for held in [stale(Teacher, false), stale(Teacher, true)] {
            for action in [
                Action::ViewCourse(held),
                Action::PublishCourse(held),
                Action::ManageContent(held),
                Action::GradeAssignment(held),
                Action::ViewGradebook(held),
                Action::ManageRoster(held),
                Action::EditCourse(held),
                Action::TransferOwnership(held),
                Action::ManageMembership {
                    access: held,
                    role: Student,
                },
            ] {
                assert_eq!(
                    authorize(&student, action),
                    Err(PolicyError::Denied),
                    "{action:?}"
                );
            }
        }
        // The same access held by an institution teacher is what it always was.
        assert_eq!(
            authorize(
                &actor(Role::Teacher),
                Action::ManageContent(stale(Teacher, true))
            ),
            Ok(())
        );
        // An assistant or a student role is a different matter and stays.
        assert_eq!(
            authorize(&student, Action::ManageContent(stale(Assistant, false))),
            Ok(())
        );
    }
}
