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
fn a_module_permission_is_held_by_the_roles_the_module_declared_and_nobody_else() {
    let action = Action::Module {
        module: "dummy",
        permission: "items.write",
        roles: &[Role::Teacher, Role::Admin],
    };
    for role in [Role::Teacher, Role::Admin] {
        assert_eq!(authorize(&actor(role), action), Ok(()), "{role:?}");
    }
    assert_eq!(
        authorize(&actor(Role::Student), action),
        Err(AccessError::Denied)
    );
    let nobody = Action::Module {
        module: "dummy",
        permission: "none",
        roles: &[],
    };
    assert_eq!(
        authorize(&actor(Role::Admin), nobody),
        Err(AccessError::Denied)
    );
}

#[test]
fn only_admins_manage_users() {
    assert_eq!(authorize(&actor(Role::Admin), Action::ManageUsers), Ok(()));
    for role in [Role::Teacher, Role::Student] {
        assert_eq!(
            authorize(&actor(role), Action::ManageUsers),
            Err(AccessError::Denied),
            "{role:?}"
        );
    }
}

#[test]
fn only_admins_manage_terms_and_groups_and_read_groups() {
    for action in [
        Action::ViewInstitutionSettings,
        Action::ManageAppDownloads,
        Action::ViewAdminReports,
        Action::ManageTerms,
        Action::ManageGroups,
        Action::ViewGroups,
    ] {
        assert_eq!(authorize(&actor(Role::Admin), action), Ok(()), "{action:?}");
        for role in [Role::Teacher, Role::Student] {
            assert_eq!(
                authorize(&actor(role), action),
                Err(AccessError::Denied),
                "{action:?} {role:?}"
            );
        }
    }
}

/// What a write action that staff may do answers in `status` (ADR-0048).
fn written_in(status: CourseStatus) -> Result<(), AccessError> {
    if status == Archived {
        Err(AccessError::Archived)
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
                        Err(AccessError::Denied),
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
                        Err(AccessError::Denied),
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
                Err(AccessError::Denied),
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
    for write in [Action::SubmitWork(archived), Action::MarkDone(archived)] {
        assert_eq!(
            authorize(&student, write),
            Err(AccessError::Archived),
            "{write:?}"
        );
    }
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
        Err(AccessError::Denied)
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
                Err(AccessError::Denied),
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
        unit_grant: None,
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
        Err(AccessError::Denied)
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
            target_is_actor: false,
        };
        assert_eq!(
            authorize(&actor(Role::Student), action),
            Err(AccessError::Denied),
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
        Err(AccessError::Denied)
    );
    assert_eq!(
        authorize(&assistant, Action::TransferOwnership(access)),
        Err(AccessError::Denied)
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
        assert_eq!(authorize(&teacher, action), Err(AccessError::Denied));
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
                Err(AccessError::Denied),
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
    assert_eq!(authorize(&student, view(Draft)), Err(AccessError::Denied));
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
            Err(AccessError::Denied),
            "{role:?}"
        );
    }
    assert_eq!(
        authorize(&actor(Role::Teacher), publish(draft(None, false))),
        Err(AccessError::Denied)
    );
}

#[test]
fn students_can_be_enrolled_in_a_draft_but_an_archived_course_takes_no_change() {
    for status in [Draft, Published, Archived] {
        let action = Action::ManageMembership {
            access: access(Some(Teacher), false, status),
            role: Student,
            target_is_actor: false,
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
                Err(AccessError::Denied),
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
        target_is_actor: false,
    };
    assert_eq!(authorize(&assistant, manage(Student)), Ok(()));
    assert_eq!(
        authorize(&assistant, manage(Teacher)),
        Err(AccessError::Denied)
    );
    assert_eq!(
        authorize(&assistant, manage(Assistant)),
        Err(AccessError::Denied)
    );
}

#[test]
fn teacher_and_owner_manage_staff_and_students() {
    for access in [member(Teacher), owner()] {
        for role in [Teacher, Assistant, Student] {
            for target_is_actor in [false, true] {
                let action = Action::ManageMembership {
                    access,
                    role,
                    target_is_actor,
                };
                assert_eq!(authorize(&actor(Role::Teacher), action), Ok(()), "{role:?}");
            }
        }
    }
}

#[test]
fn non_members_cannot_manage_members_but_admins_can() {
    for role in [Teacher, Assistant, Student] {
        let action = Action::ManageMembership {
            access: non_member(),
            role,
            target_is_actor: false,
        };
        assert_eq!(
            authorize(&actor(Role::Teacher), action),
            Err(AccessError::Denied)
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
            Err(AccessError::Denied),
            "{role:?}"
        );
    }
    assert_eq!(authorize(&actor(Role::Admin), action), Ok(()));
}

#[test]
fn staff_and_admins_read_the_gradebook_in_every_status_and_grade_and_publish_but_not_in_an_archived_one()
 {
    type Make = fn(CourseAccess) -> Action;
    let actions: [(Make, bool); 4] = [
        (Action::GradeAssignment, true),
        (Action::PublishGrades, true),
        (Action::ManageGradebook, true),
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
                        Err(AccessError::Denied),
                        "{action:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn only_a_student_of_a_published_course_saves_submits_marks_done_and_reads_their_own_work() {
    let actions: [fn(CourseAccess) -> Action; 3] = [
        Action::SubmitWork,
        Action::MarkDone,
        Action::ViewOwnSubmission,
    ];
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
            Err(AccessError::Denied),
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
                    Err(AccessError::Denied),
                    "{role:?} {who:?}"
                );
            }
        }
        assert_eq!(
            authorize(&actor(Role::Admin), make(access(None, false, Published))),
            Err(AccessError::Denied)
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
                    Err(AccessError::Denied),
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
        Err(AccessError::Denied),
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
            Err(AccessError::Denied),
            "{role:?}"
        );
    }
    // An admin has no membership and is not a student.
    assert_eq!(
        authorize(&actor(Role::Admin), action(None, false, Published)),
        Err(AccessError::Denied)
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
                Err(AccessError::Denied),
                "{role:?} {status:?}"
            );
            assert_eq!(
                authorize(&actor(Role::Teacher), action),
                Err(AccessError::Denied),
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
            Action::ManageGradebook(held),
            Action::ManageRoster(held),
            Action::EditCourse(held),
            Action::TransferOwnership(held),
            Action::ManageMembership {
                access: held,
                role: Student,
                target_is_actor: false,
            },
        ] {
            assert_eq!(
                authorize(&student, action),
                Err(AccessError::Denied),
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

fn plugin(scopes: &[Scope]) -> PluginCaller {
    PluginCaller::new(
        Uuid::now_v7(),
        "test".to_owned(),
        scopes.to_vec(),
        CourseScope::All,
    )
}

fn plugin_in(scopes: &[Scope], courses: &[Uuid]) -> PluginCaller {
    PluginCaller::new(
        Uuid::now_v7(),
        "test".to_owned(),
        scopes.to_vec(),
        CourseScope::Only(courses.to_vec()),
    )
}

#[test]
fn a_plugin_may_do_only_what_it_was_granted() {
    let reader = plugin(&[Scope::SubmissionsRead]);
    assert_eq!(authorize_plugin(&reader, Scope::SubmissionsRead), Ok(()));
    assert_eq!(
        authorize_plugin(&reader, Scope::ReportsWrite),
        Err(AccessError::Denied)
    );
    let nothing = plugin(&[]);
    for scope in Scope::ALL {
        assert_eq!(authorize_plugin(&nothing, scope), Err(AccessError::Denied));
    }
}

#[test]
fn scopes_have_the_names_the_manifest_and_the_cli_use() {
    for (text, scope) in [
        ("submissions:read", Scope::SubmissionsRead),
        ("reports:write", Scope::ReportsWrite),
        ("settings:read", Scope::SettingsRead),
        ("assignments:read", Scope::AssignmentsRead),
        ("roster:read", Scope::RosterRead),
        ("grades:read", Scope::GradesRead),
        ("quizzes:read", Scope::QuizzesRead),
        ("users:contact", Scope::UsersContact),
        ("announcements:read", Scope::AnnouncementsRead),
        ("mail:send", Scope::MailSend),
    ] {
        assert_eq!(text.parse::<Scope>(), Ok(scope));
        assert_eq!(scope.as_str(), text);
    }
    assert!("submissions:write".parse::<Scope>().is_err());
}

#[test]
fn a_plugin_reaches_only_the_courses_it_was_granted() {
    let (mine, other) = (Uuid::now_v7(), Uuid::now_v7());
    let one = plugin_in(&[Scope::GradesRead], &[mine]);
    assert_eq!(
        authorize_plugin_course(&one, Scope::GradesRead, mine),
        Ok(())
    );
    assert_eq!(
        authorize_plugin_course(&one, Scope::GradesRead, other),
        Err(AccessError::Denied)
    );
    // The scope is checked too: a course alone grants nothing.
    assert_eq!(
        authorize_plugin_course(&one, Scope::RosterRead, mine),
        Err(AccessError::Denied)
    );
    let all = plugin(&[Scope::GradesRead]);
    assert_eq!(
        authorize_plugin_course(&all, Scope::GradesRead, other),
        Ok(())
    );
    let none = plugin_in(&[Scope::GradesRead], &[]);
    assert_eq!(
        authorize_plugin_course(&none, Scope::GradesRead, mine),
        Err(AccessError::Denied)
    );
}

#[test]
fn a_list_is_about_the_requested_course_or_every_course_the_plugin_reaches() {
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    let scoped = plugin_in(&[Scope::RosterRead], &[a, b]);
    assert_eq!(
        plugin_course_filter(&scoped, Scope::RosterRead, None),
        Ok(Some(vec![a, b]))
    );
    assert_eq!(
        plugin_course_filter(&scoped, Scope::RosterRead, Some(a)),
        Ok(Some(vec![a]))
    );
    assert_eq!(
        plugin_course_filter(&scoped, Scope::RosterRead, Some(Uuid::now_v7())),
        Err(AccessError::Denied)
    );
    assert_eq!(
        plugin_course_filter(&scoped, Scope::GradesRead, None),
        Err(AccessError::Denied)
    );
    let all = plugin(&[Scope::RosterRead]);
    assert_eq!(
        plugin_course_filter(&all, Scope::RosterRead, None),
        Ok(None)
    );
}

fn route<'a>(roles: &'a [Role], course_roles: &'a [CourseRole]) -> PluginRoute<'a> {
    PluginRoute {
        roles,
        course_roles,
    }
}

#[test]
fn a_plugin_route_is_open_to_the_institution_roles_it_lists_and_no_others() {
    let open = route(&[Role::Teacher], &[]);
    assert_eq!(
        authorize_plugin_route(&actor(Role::Teacher), open, None),
        Ok(())
    );
    // An administrator is let in only by name.
    for role in [Role::Admin, Role::Student] {
        assert_eq!(
            authorize_plugin_route(&actor(role), open, None),
            Err(AccessError::Denied)
        );
    }
    let nobody = route(&[], &[]);
    for role in [Role::Admin, Role::Teacher, Role::Student] {
        assert_eq!(
            authorize_plugin_route(&actor(role), nobody, None),
            Err(AccessError::Denied)
        );
    }
}

#[test]
fn a_plugin_route_with_course_roles_asks_for_one_in_that_course() {
    let staff = route(
        &[Role::Admin, Role::Teacher, Role::Student],
        &[Teacher, Assistant],
    );
    let teacher = actor(Role::Teacher);
    assert_eq!(
        authorize_plugin_route(&teacher, staff, Some(member(Teacher))),
        Ok(())
    );
    assert_eq!(
        authorize_plugin_route(&teacher, staff, Some(member(Assistant))),
        Ok(())
    );
    assert_eq!(
        authorize_plugin_route(&teacher, staff, Some(owner())),
        Ok(())
    );
    // A student of the course, a stranger, and a call with no course are refused.
    assert_eq!(
        authorize_plugin_route(&actor(Role::Student), staff, Some(member(Student))),
        Err(AccessError::Denied)
    );
    assert_eq!(
        authorize_plugin_route(&teacher, staff, Some(non_member())),
        Err(AccessError::Denied)
    );
    assert_eq!(
        authorize_plugin_route(&teacher, staff, None),
        Err(AccessError::Denied)
    );
    // An institution student is never a course teacher, whatever the roster says (ADR-0037).
    assert_eq!(
        authorize_plugin_route(&actor(Role::Student), staff, Some(member(Teacher))),
        Err(AccessError::Denied)
    );
    // An administrator who is listed runs every course without a membership.
    assert_eq!(
        authorize_plugin_route(&actor(Role::Admin), staff, Some(non_member())),
        Ok(())
    );
    assert_eq!(
        authorize_plugin_route(&actor(Role::Admin), staff, None),
        Ok(())
    );
}

#[test]
fn the_user_a_plugin_is_told_of_is_the_actor_and_their_institution_role() {
    let teacher = actor(Role::Teacher);
    let user = plugin_user(&teacher);
    assert_eq!((user.id, user.role), (teacher.user_id(), Role::Teacher));
}

fn unit_access(grant: UnitGrant, status: CourseStatus) -> CourseAccess {
    CourseAccess {
        role: None,
        is_owner: false,
        status,
        unit_grant: Some(grant),
    }
}

type CourseAction = fn(CourseAccess) -> Action;

/// Each course action with the ability that lets a unit grant through (`None`: any grant).
fn unit_actions() -> Vec<(Option<UnitAbility>, Vec<CourseAction>)> {
    use UnitAbility::{EditContent, Grade, ManageCourses, ManageRoster, ViewGrades};
    vec![
        (
            None,
            vec![Action::ViewCourse, Action::ManageContent, |access| {
                Action::ViewContent {
                    access,
                    module: Release {
                        published: false,
                        publish_at: None,
                    },
                    submodule: None,
                    item: None,
                    at: DateTime::UNIX_EPOCH,
                }
            }],
        ),
        (
            Some(ViewGrades),
            vec![Action::ViewGradebook, Action::ViewSubmissions],
        ),
        (Some(EditContent), vec![Action::EditContent]),
        (
            Some(Grade),
            vec![
                Action::GradeAssignment,
                Action::PublishGrades,
                Action::ManageGradebook,
            ],
        ),
        (
            Some(ManageRoster),
            vec![
                Action::ManageRoster,
                Action::ChangeRoster,
                |access| Action::ManageMembership {
                    access,
                    role: Student,
                    target_is_actor: false,
                },
                |access| Action::ManageMembership {
                    access,
                    role: Assistant,
                    target_is_actor: false,
                },
                |access| Action::ManageMembership {
                    access,
                    role: Teacher,
                    target_is_actor: false,
                },
                Action::LinkGroups,
                Action::ChangeGroupLinks,
            ],
        ),
        (
            Some(ManageCourses),
            vec![
                Action::EditCourse,
                Action::PublishCourse,
                Action::ArchiveCourse,
            ],
        ),
    ]
}

#[test]
fn a_unit_grant_lets_through_what_its_abilities_allow() {
    let teacher = actor(Role::Teacher);
    let unit = Uuid::now_v7();
    for (needs, actions) in unit_actions() {
        let with = match needs {
            Some(ability) => UnitGrant::new(unit, [ability]),
            None => UnitGrant::new(unit, []),
        };
        let without = UnitGrant::new(
            unit,
            UnitAbility::ALL
                .into_iter()
                .filter(|ability| Some(*ability) != needs),
        );
        for action in actions {
            let granted = action(unit_access(with, Published));
            assert_eq!(authorize(&teacher, granted), Ok(()), "{granted:?}");
            let refused = action(unit_access(without, Published));
            let expected = if needs.is_none() {
                Ok(())
            } else {
                Err(AccessError::Denied)
            };
            assert_eq!(authorize(&teacher, refused), expected, "{refused:?}");
        }
    }
    // View needs no ability, and sees a draft; no grant at all sees nothing.
    let draft = unit_access(UnitGrant::new(unit, []), Draft);
    assert_eq!(authorize(&teacher, Action::ViewCourse(draft)), Ok(()));
    assert_eq!(
        authorize(&teacher, Action::ViewCourse(non_member())),
        Err(AccessError::Denied)
    );
}

#[test]
fn a_unit_grant_never_deletes_transfers_or_acts_as_a_student() {
    let teacher = actor(Role::Teacher);
    let access = unit_access(UnitGrant::full(Uuid::now_v7()), Published);
    for action in [
        Action::DeleteCourse(access),
        Action::TransferOwnership(access),
        Action::SubmitWork(access),
        Action::MarkDone(access),
        Action::ViewOwnGrades(access),
        Action::ViewOwnSubmission(access),
    ] {
        assert_eq!(
            authorize(&teacher, action),
            Err(AccessError::Denied),
            "{action:?}"
        );
    }
}

#[test]
fn a_student_holds_no_unit_grant() {
    let student = actor(Role::Student);
    let grant = UnitGrant::full(Uuid::now_v7());
    let access = unit_access(grant, Draft);
    assert_eq!(
        authorize(&student, Action::ViewCourse(access)),
        Err(AccessError::Denied)
    );
    assert_eq!(
        authorize(&student, Action::EditContent(access)),
        Err(AccessError::Denied)
    );
    assert_eq!(
        authorize(&student, Action::ViewUnit(Some(grant))),
        Err(AccessError::Denied)
    );
    assert_eq!(implicit_access(&student, access), None);
}

#[test]
fn unit_actions_need_the_grant_or_admin() {
    let (admin, teacher, student) = (
        actor(Role::Admin),
        actor(Role::Teacher),
        actor(Role::Student),
    );
    let unit = Uuid::now_v7();
    let viewer = UnitGrant::new(unit, []);
    assert_eq!(
        authorize(&teacher, Action::ViewUnit(None)),
        Err(AccessError::Denied)
    );
    assert_eq!(authorize(&admin, Action::ViewUnit(None)), Ok(()));
    assert_eq!(authorize(&teacher, Action::ViewUnit(Some(viewer))), Ok(()));
    for (ability, action) in [
        (
            UnitAbility::ManageUnits,
            Action::ManageUnits as fn(Option<UnitGrant>) -> Action,
        ),
        (UnitAbility::ManageCourses, Action::ManageUnitCourses),
    ] {
        assert_eq!(authorize(&admin, action(None)), Ok(()));
        assert_eq!(authorize(&teacher, action(None)), Err(AccessError::Denied));
        assert_eq!(
            authorize(&teacher, action(Some(viewer))),
            Err(AccessError::Denied)
        );
        assert_eq!(
            authorize(&teacher, action(Some(UnitGrant::new(unit, [ability])))),
            Ok(())
        );
    }
    assert_eq!(authorize(&admin, Action::ManageUnitRoleAbilities), Ok(()));
    for who in [&teacher, &student] {
        assert_eq!(
            authorize(who, Action::ManageUnitRoleAbilities),
            Err(AccessError::Denied)
        );
    }
    for who in [&admin, &teacher, &student] {
        assert_eq!(authorize(who, Action::ViewOwnUnits), Ok(()));
    }
}

#[test]
fn an_archived_course_refuses_unit_writes() {
    let teacher = actor(Role::Teacher);
    let access = unit_access(UnitGrant::full(Uuid::now_v7()), Archived);
    assert_eq!(
        authorize(&teacher, Action::EditContent(access)),
        Err(AccessError::Archived)
    );
    assert_eq!(authorize(&teacher, Action::ViewCourse(access)), Ok(()));
}

#[test]
fn implicit_access_names_the_way_in() {
    let grant = UnitGrant::full(Uuid::now_v7());
    assert_eq!(
        implicit_access(&actor(Role::Admin), owner()),
        Some(Via::Admin)
    );
    assert_eq!(
        implicit_access(&actor(Role::Teacher), unit_access(grant, Published)),
        Some(Via::Unit)
    );
    let on_the_course = CourseAccess {
        role: Some(Teacher),
        ..unit_access(grant, Published)
    };
    assert_eq!(implicit_access(&actor(Role::Teacher), on_the_course), None);
    let owning = CourseAccess {
        is_owner: true,
        ..unit_access(grant, Published)
    };
    assert_eq!(implicit_access(&actor(Role::Teacher), owning), None);
    assert_eq!(implicit_access(&actor(Role::Teacher), non_member()), None);
    // An assistant is staff: not implicit either.
    let assisting = CourseAccess {
        role: Some(Assistant),
        ..unit_access(grant, Published)
    };
    assert_eq!(implicit_access(&actor(Role::Teacher), assisting), None);
    // A grantee who studies in the course writes through the grant: audited.
    let studying = CourseAccess {
        role: Some(Student),
        ..unit_access(grant, Published)
    };
    assert_eq!(
        implicit_access(&actor(Role::Teacher), studying),
        Some(Via::Unit)
    );
}

/// `manage_roster` from a grant lets a manager add others at any role, never change their own row:
/// otherwise they could make themselves teacher past the admin's switches (ADR-0094).
#[test]
fn a_unit_grant_alone_never_manages_the_actors_own_membership() {
    let teacher = actor(Role::Teacher);
    let grant = UnitGrant::new(Uuid::now_v7(), [UnitAbility::ManageRoster]);
    let manage = |access, role, target_is_actor| Action::ManageMembership {
        access,
        role,
        target_is_actor,
    };
    for role in [Teacher, Assistant, Student] {
        let grant_only = unit_access(grant, Published);
        assert_eq!(
            authorize(&teacher, manage(grant_only, role, true)),
            Err(AccessError::Denied),
            "{role:?}"
        );
        assert_eq!(
            authorize(&teacher, manage(grant_only, role, false)),
            Ok(()),
            "{role:?}"
        );
        // A student of the course is not staff: the grant is still all they have.
        let studying = CourseAccess {
            role: Some(Student),
            ..grant_only
        };
        assert_eq!(
            authorize(&teacher, manage(studying, role, true)),
            Err(AccessError::Denied),
            "{role:?}"
        );
        // Course staff keep what their place on the course gives them.
        let teaching = CourseAccess {
            role: Some(Teacher),
            ..grant_only
        };
        assert_eq!(
            authorize(&teacher, manage(teaching, role, true)),
            Ok(()),
            "{role:?}"
        );
        // An admin is unaffected.
        assert_eq!(
            authorize(&actor(Role::Admin), manage(non_member(), role, true)),
            Ok(()),
            "{role:?}"
        );
    }
    // An assistant may change a student's row through the course, not their own staff role
    // through the grant.
    let assisting = CourseAccess {
        role: Some(Assistant),
        ..unit_access(grant, Published)
    };
    assert_eq!(
        authorize(&teacher, manage(assisting, Student, true)),
        Ok(())
    );
    assert_eq!(
        authorize(&teacher, manage(assisting, Teacher, true)),
        Err(AccessError::Denied)
    );
    assert_eq!(
        authorize(&teacher, manage(assisting, Teacher, false)),
        Ok(())
    );
}

#[test]
fn course_can_answers_as_the_api_would() {
    let teacher = actor(Role::Teacher);
    let grant = UnitGrant::new(
        Uuid::now_v7(),
        [UnitAbility::ManageRoster, UnitAbility::Grade],
    );
    let can = course_can(&teacher, unit_access(grant, Published));
    assert_eq!(
        can,
        CourseCan {
            view_grades: false,
            edit_content: false,
            grade: true,
            manage_roster: true,
            manage_course: false,
        }
    );
    let archived = course_can(&teacher, unit_access(grant, Archived));
    // Every write control reads as refused, `manage_roster` (the write `ChangeRoster`) included.
    assert!(!archived.grade && !archived.manage_roster);
    let all = course_can(&actor(Role::Admin), non_member());
    assert!(
        all.view_grades && all.edit_content && all.grade && all.manage_roster && all.manage_course
    );
}

#[test]
fn a_unit_grant_keeps_its_abilities_in_order() {
    let unit = Uuid::now_v7();
    let grant = UnitGrant::new(
        unit,
        [
            UnitAbility::ManageUnits,
            UnitAbility::ViewGrades,
            UnitAbility::ViewGrades,
        ],
    );
    assert_eq!(
        grant.abilities(),
        vec![UnitAbility::ViewGrades, UnitAbility::ManageUnits]
    );
    assert_eq!(UnitGrant::full(unit).abilities(), UnitAbility::ALL.to_vec());
    assert!(!grant.has(UnitAbility::Grade));
    for ability in UnitAbility::ALL {
        assert_eq!(ability.as_str().parse::<UnitAbility>(), Ok(ability));
    }
    assert_eq!("viewer".parse::<UnitRole>(), Ok(UnitRole::Viewer));
}

#[test]
fn unit_role_allowed_is_for_teachers_and_admins() {
    assert!(unit_role_allowed(Role::Teacher));
    assert!(unit_role_allowed(Role::Admin));
    assert!(!unit_role_allowed(Role::Student));
}
