//! The only place that decides who may do what (ADR-0004, ADR-0019). Deny by default.

use chrono::{DateTime, Utc};

use crate::actor::Actor;
use crate::identity::Role;
use crate::text_enum::text_enum;

/// A user's role inside one course (`teaching.memberships.role`). The owner is a `Teacher`
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
    /// Owner, teacher or assistant: may see every status and manage students.
    fn is_staff(self) -> bool {
        self.is_owner || matches!(self.role, Some(CourseRole::Teacher | CourseRole::Assistant))
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
    /// Create, list, change, disable and reset users, institution-wide.
    ManageUsers,
    /// Create a course; the creator becomes its owner.
    CreateCourse,
    /// List the courses one belongs to. Which ones is decided per course by `ViewCourse`.
    ViewOwnCourses,
    /// Read a course and its roster. A student sees it only once it is published.
    ViewCourse(CourseAccess),
    /// Move a course from draft to published (ADR-0019).
    PublishCourse(CourseAccess),
    /// Add, change or remove a member who holds (or will hold) `role`. Ignores the course
    /// status, so students can be enrolled in a draft.
    ManageMembership {
        access: CourseAccess,
        role: CourseRole,
    },
    DeleteCourse(CourseAccess),
    TransferOwnership(CourseAccess),
    /// View drafts, edit and publish content (ADR-0004). Ignores the course status.
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
    /// Save a draft grade (ADR-0028). Staff and admins; ignores the course status.
    GradeAssignment(CourseAccess),
    /// Copy draft grades to published (ADR-0028). Staff and admins; ignores the course status.
    PublishGrades(CourseAccess),
    /// Read the staff gradebook of a course, with every draft value (ADR-0028).
    ViewGradebook(CourseAccess),
    /// Read one's own published grades: only a student of a published course (ADR-0028). Staff
    /// and admins use the gradebook instead.
    ViewOwnGrades(CourseAccess),
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

/// True when the actor acts in any course as its implicit owner, without a membership. Such
/// actions are audited.
pub fn implicit_course_owner(actor: &Actor) -> bool {
    actor.role() == Role::Admin
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not allowed")]
pub struct PolicyError;

/// Allows or denies `action`. The match has no wildcard: a new action does not compile
/// until someone decides who may do it.
pub fn authorize(actor: &Actor, action: Action) -> Result<(), PolicyError> {
    let is_admin = actor.role() == Role::Admin;
    let allowed = match action {
        Action::ManageOwnAccount => true,
        Action::ManageUsers => is_admin,
        Action::CreateCourse => matches!(actor.role(), Role::Admin | Role::Teacher),
        Action::ViewOwnCourses => true,
        Action::ViewEditorProfiles => true,
        Action::ViewCourse(access) => {
            is_admin
                || access.is_staff()
                || (access.role == Some(CourseRole::Student)
                    && access.status == CourseStatus::Published)
        }
        Action::PublishCourse(access) => is_admin || access.is_owner_or_teacher(),
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
        | Action::GradeAssignment(access)
        | Action::PublishGrades(access)
        | Action::ViewGradebook(access) => is_admin || access.is_staff(),
        Action::ViewOwnGrades(access) => {
            access.role == Some(CourseRole::Student) && access.status == CourseStatus::Published
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
                    && access.status == CourseStatus::Published
                    && [Some(module), submodule, item]
                        .into_iter()
                        .flatten()
                        .all(|release| release.is_open_at(at)))
        }
    };
    if allowed { Ok(()) } else { Err(PolicyError) }
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
                Err(PolicyError),
                "{role:?}"
            );
        }
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
            Err(PolicyError)
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
                Err(PolicyError),
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
            Err(PolicyError)
        );
        assert_eq!(
            authorize(&assistant, Action::TransferOwnership(access)),
            Err(PolicyError)
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
            assert_eq!(authorize(&teacher, action), Err(PolicyError));
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
                    Err(PolicyError),
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
    fn student_member_sees_only_published_courses() {
        let student = actor(Role::Student);
        let view = |status| Action::ViewCourse(access(Some(Student), false, status));
        assert_eq!(authorize(&student, view(Published)), Ok(()));
        assert_eq!(authorize(&student, view(Draft)), Err(PolicyError));
        assert_eq!(authorize(&student, view(Archived)), Err(PolicyError));
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
                Err(PolicyError),
                "{role:?}"
            );
        }
        assert_eq!(
            authorize(&actor(Role::Teacher), publish(draft(None, false))),
            Err(PolicyError)
        );
    }

    #[test]
    fn managing_members_ignores_the_course_status() {
        for status in [Draft, Published, Archived] {
            let action = Action::ManageMembership {
                access: access(Some(Teacher), false, status),
                role: Student,
            };
            assert_eq!(
                authorize(&actor(Role::Teacher), action),
                Ok(()),
                "{status:?}"
            );
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
        assert_eq!(authorize(&assistant, manage(Teacher)), Err(PolicyError));
        assert_eq!(authorize(&assistant, manage(Assistant)), Err(PolicyError));
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
            assert_eq!(authorize(&actor(Role::Teacher), action), Err(PolicyError));
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

        for status in [Draft, Archived] {
            let closed = access(Some(Student), false, status);
            assert!(
                !allowed(view(closed, open(), Some(open()), Some(open()))),
                "{status:?}"
            );
        }
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
                Err(PolicyError),
                "{role:?}"
            );
        }
        assert_eq!(authorize(&actor(Role::Admin), action), Ok(()));
    }

    #[test]
    fn staff_and_admins_grade_publish_and_read_the_gradebook_in_every_status() {
        let actions: [fn(CourseAccess) -> Action; 3] = [
            Action::GradeAssignment,
            Action::PublishGrades,
            Action::ViewGradebook,
        ];
        for make in actions {
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
                            Err(PolicyError),
                            "{action:?}"
                        );
                    }
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
        for status in [Draft, Archived] {
            assert_eq!(
                authorize(&actor(Role::Student), action(Some(Student), false, status)),
                Err(PolicyError),
                "{status:?}"
            );
        }
        for (role, is_owner) in [
            (Some(Teacher), true),
            (Some(Teacher), false),
            (Some(Assistant), false),
            (None, false),
        ] {
            assert_eq!(
                authorize(&actor(Role::Teacher), action(role, is_owner, Published)),
                Err(PolicyError),
                "{role:?}"
            );
        }
        // An admin has no membership and is not a student.
        assert_eq!(
            authorize(&actor(Role::Admin), action(None, false, Published)),
            Err(PolicyError)
        );
    }

    #[test]
    fn staff_and_admins_manage_content_in_every_course_status_and_students_never_do() {
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
                    Err(PolicyError),
                    "{role:?} {status:?}"
                );
                assert_eq!(
                    authorize(&actor(Role::Teacher), action),
                    Err(PolicyError),
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
}
