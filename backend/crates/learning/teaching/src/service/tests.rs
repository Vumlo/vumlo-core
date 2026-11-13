use crate::tests::db_test;
use test_support::wait_until_blocked;

use super::*;
use auth::{self, Email, HashedPassword, NewUser, PersonName};
use common::Role;

async fn actor(pool: &sqlx::PgPool, hash: &HashedPassword, email: &str, role: Role) -> Actor {
    let mut tx = pool.begin().await.unwrap();
    let user = auth::create_user_unaudited_for_tests(
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
    Actor::for_tests(user.id, role, Uuid::now_v7())
}

/// The owner promotes a student to teacher and holds its transaction open; an assistant
/// removes that student meanwhile. The removal must wait for the course lock and then see the
/// new role, which an assistant may not remove. Without the lock it would read the old role
/// and delete the new teacher.
#[tokio::test]
async fn member_writes_on_one_course_see_each_others_committed_roles() {
    db_test(async |pool| {
        let hash = auth::hash_password("irrelevant").await.unwrap();
        let owner = actor(&pool, &hash, "owner@x.edu", Role::Teacher).await;
        let assistant = actor(&pool, &hash, "assistant@x.edu", Role::Student).await;
        let promoted = actor(&pool, &hash, "promoted@x.edu", Role::Teacher).await;

        let mut tx = pool.begin().await.unwrap();
        let course = create_course(
            &mut tx,
            &owner,
            CourseTitle::parse("Algebra").unwrap(),
            None,
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
        let hash = auth::hash_password("irrelevant").await.unwrap();
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
        let hash = auth::hash_password("irrelevant").await.unwrap();
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
                status: CourseStatus::Draft,
                unit_grant: None,
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
    let hash = auth::hash_password("irrelevant").await.unwrap();
    let admin = actor(&pool, &hash, "admin@x.edu", Role::Admin).await;
    let owner = actor(&pool, &hash, "owner@x.edu", Role::Teacher).await;
    let student = actor(&pool, &hash, "student@x.edu", Role::Student).await;
    let mut tx = pool.begin().await.unwrap();
    let group = org::seed_group(&mut tx, &admin, "7-A", &[student.user_id()])
        .await
        .unwrap();
    let course = create_course(
        &mut tx,
        &owner,
        CourseTitle::parse("Algebra").unwrap(),
        None,
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

// ---------------------------------------------------------------------------------------------
// Courses in units (ADR-0094)
// ---------------------------------------------------------------------------------------------

/// Units A > B and A > S, a draft course K in B made by an admin, Pat a `viewer` of A and Max a
/// `manager` of B. Neither is on K. Tess is a teacher with no unit role.
struct Tree {
    pool: sqlx::PgPool,
    admin: Actor,
    pat: Actor,
    max: Actor,
    tess: Actor,
    a: Uuid,
    b: Uuid,
    s: Uuid,
    k: Uuid,
}

async fn tree(pool: sqlx::PgPool) -> Tree {
    let hash = auth::hash_password("irrelevant").await.unwrap();
    let admin = actor(&pool, &hash, "admin@x.edu", Role::Admin).await;
    let pat = actor(&pool, &hash, "pat@x.edu", Role::Teacher).await;
    let max = actor(&pool, &hash, "max@x.edu", Role::Teacher).await;
    let tess = actor(&pool, &hash, "tess@x.edu", Role::Teacher).await;
    let mut tx = pool.begin().await.unwrap();
    let unit = async |tx: &mut sqlx::PgConnection, name: &str, parent| {
        org::create_unit(tx, &admin, org::OrgName::parse(name).unwrap(), parent)
            .await
            .unwrap()
            .id
    };
    let a = unit(&mut tx, "A", None).await;
    let b = unit(&mut tx, "B", Some(a)).await;
    let s = unit(&mut tx, "S", Some(a)).await;
    org::set_unit_role(&mut tx, &admin, a, pat.user_id(), access::UnitRole::Viewer)
        .await
        .unwrap();
    org::set_unit_role(&mut tx, &admin, b, max.user_id(), access::UnitRole::Manager)
        .await
        .unwrap();
    let k = create_course(
        &mut tx,
        &admin,
        CourseTitle::parse("K").unwrap(),
        None,
        Some(b),
    )
    .await
    .unwrap()
    .id;
    tx.commit().await.unwrap();
    Tree {
        pool,
        admin,
        pat,
        max,
        tess,
        a,
        b,
        s,
        k,
    }
}

impl Tree {
    async fn conn(&self) -> sqlx::pool::PoolConnection<sqlx::Postgres> {
        self.pool.acquire().await.unwrap()
    }

    /// How the actor stands to K, read afresh.
    async fn access(&self, who: &Actor) -> CourseAccess {
        let mut conn = self.conn().await;
        course_access(&mut conn, who, self.k).await.unwrap()
    }

    /// Replaces the switches of the `viewer` role.
    async fn viewer_may(&self, abilities: Vec<access::UnitAbility>) {
        let mut tx = self.pool.begin().await.unwrap();
        org::set_unit_role_abilities(&mut tx, &self.admin, access::UnitRole::Viewer, abilities)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    async fn audit_vias(&self, action: &str) -> Vec<(Option<String>, Option<String>)> {
        sqlx::query!(
            "SELECT meta->>'via' AS via, meta->>'unit_id' AS unit_id
             FROM events.audit_log WHERE action = $1 ORDER BY at, id",
            action
        )
        .fetch_all(&self.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.via, row.unit_id))
        .collect()
    }
}

#[tokio::test]
async fn a_unit_viewer_sees_a_course_below_in_every_status() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let mut conn = t.conn().await;
        // K is a draft, Pat is on no roster and owns nothing.
        let course = get_course(&mut conn, &t.pat, t.k).await.unwrap();
        assert_eq!(course.unit_id, Some(t.b));
        let access = course_access(&mut conn, &t.pat, t.k).await.unwrap();
        assert_eq!(access.role, None);
        assert_eq!(access.unit_grant.map(|grant| grant.unit_id), Some(t.b));
        drop(conn);

        let mut tx = t.pool.begin().await.unwrap();
        publish_course(&mut tx, &t.admin, t.k).await.unwrap();
        archive_course(&mut tx, &t.admin, t.k).await.unwrap();
        tx.commit().await.unwrap();
        let mut conn = t.conn().await;
        let course = get_course(&mut conn, &t.pat, t.k).await.unwrap();
        assert_eq!(course.status, CourseStatus::Archived);

        // The grant is on A: a teacher with no role there does not see K.
        let outcome = get_course(&mut conn, &t.tess, t.k).await;
        assert!(
            matches!(outcome, Err(TeachingError::CourseNotFound)),
            "{outcome:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn a_unit_viewer_cannot_edit_and_a_manager_with_edit_content_can() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let rename = || CourseUpdate {
            title: Some("Renamed".to_owned()),
            term_id: None,
        };
        let edit_content = |access| access::authorize(&t.pat, Action::EditContent(access));
        let mut tx = t.pool.begin().await.unwrap();
        let outcome = update_course(&mut tx, &t.pat, t.k, rename()).await;
        assert!(
            matches!(outcome, Err(TeachingError::Forbidden(_))),
            "{outcome:?}"
        );
        tx.rollback().await.unwrap();
        let access = t.access(&t.pat).await;
        assert!(edit_content(access).is_err());

        // `edit_content` switched on: the very next read has it. Editing the course itself is
        // `manage_courses`, which the viewer still lacks.
        t.viewer_may(vec![
            access::UnitAbility::ViewGrades,
            access::UnitAbility::EditContent,
        ])
        .await;
        let access = t.access(&t.pat).await;
        assert!(edit_content(access).is_ok());
        let mut tx = t.pool.begin().await.unwrap();
        assert!(matches!(
            update_course(&mut tx, &t.pat, t.k, rename()).await,
            Err(TeachingError::Forbidden(_))
        ));
        tx.rollback().await.unwrap();

        // A manager of B has `manage_courses` by default and may rename it.
        let mut tx = t.pool.begin().await.unwrap();
        let course = update_course(&mut tx, &t.max, t.k, rename()).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(course.title, "Renamed");
        assert_eq!(
            t.audit_vias("course.update").await,
            [(Some("unit".to_owned()), Some(t.b.to_string()))]
        );
    })
    .await;
}

#[tokio::test]
async fn creating_in_a_unit_needs_manage_courses() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let create = async |who: &Actor, unit| {
            let mut tx = t.pool.begin().await.unwrap();
            let outcome =
                create_course(&mut tx, who, CourseTitle::parse("New").unwrap(), None, unit).await;
            if outcome.is_ok() {
                tx.commit().await.unwrap();
            }
            outcome
        };
        for who in [&t.tess, &t.pat] {
            let outcome = create(who, Some(t.b)).await;
            assert!(
                matches!(outcome, Err(TeachingError::Forbidden(_))),
                "{outcome:?}"
            );
        }
        let course = create(&t.max, Some(t.b)).await.unwrap();
        assert_eq!(course.unit_id, Some(t.b));
        // A manager of B has nothing on S, and a course with no unit needs no grant.
        assert!(matches!(
            create(&t.max, Some(t.s)).await,
            Err(TeachingError::Forbidden(_))
        ));
        assert_eq!(create(&t.tess, None).await.unwrap().unit_id, None);
        // The creator is the owner: nothing is audited for the manager's own course.
        assert!(
            t.audit_vias("course.create")
                .await
                .iter()
                .all(|(via, _)| via.as_deref() != Some("unit"))
        );
        let outcome = create(&t.admin, Some(Uuid::now_v7())).await;
        assert!(
            matches!(outcome, Err(TeachingError::UnitNotValid)),
            "{outcome:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn an_archived_course_moves_between_units() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let mut tx = t.pool.begin().await.unwrap();
        archive_course(&mut tx, &t.admin, t.k).await.unwrap();
        let moved = set_course_unit(&mut tx, &t.admin, t.k, Some(t.a))
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(moved.unit_id, Some(t.a));
        assert_eq!(moved.status, CourseStatus::Archived);

        let mut tx = t.pool.begin().await.unwrap();
        let moved = set_course_unit(&mut tx, &t.admin, t.k, None).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(moved.unit_id, None);
    })
    .await;
}

#[tokio::test]
async fn moving_needs_the_grant_on_both_units() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let set = async |who: &Actor, course, unit| {
            let mut tx = t.pool.begin().await.unwrap();
            let outcome = set_course_unit(&mut tx, who, course, unit).await;
            if outcome.is_ok() {
                tx.commit().await.unwrap();
            }
            outcome
        };
        let forbidden = |outcome: Result<Course, TeachingError>| {
            assert!(
                matches!(outcome, Err(TeachingError::Forbidden(_))),
                "{outcome:?}"
            );
        };
        // Max manages B only: not into S, and Pat (a viewer) cannot take it anywhere.
        forbidden(set(&t.max, t.k, Some(t.s)).await);
        forbidden(set(&t.pat, t.k, Some(t.b)).await);
        forbidden(set(&t.pat, t.k, None).await);
        // Someone who cannot see the course does not learn it exists.
        let outcome = set(&t.tess, t.k, None).await;
        assert!(
            matches!(outcome, Err(TeachingError::CourseNotFound)),
            "{outcome:?}"
        );

        // Out of B needs the old grant only.
        assert_eq!(set(&t.max, t.k, None).await.unwrap().unit_id, None);
        // Nobody holds a grant on a course with no unit: Max sees nothing now.
        let outcome = set(&t.max, t.k, Some(t.b)).await;
        assert!(
            matches!(outcome, Err(TeachingError::CourseNotFound)),
            "{outcome:?}"
        );
        assert_eq!(
            set(&t.admin, t.k, Some(t.b)).await.unwrap().unit_id,
            Some(t.b)
        );

        // A plain teacher moving their own course into a unit needs the grant on that unit.
        let mut tx = t.pool.begin().await.unwrap();
        let own = create_course(
            &mut tx,
            &t.tess,
            CourseTitle::parse("Own").unwrap(),
            None,
            None,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        forbidden(set(&t.tess, own.id, Some(t.b)).await);
        // ... and may not take it out of a unit they hold nothing on either.
        set(&t.admin, own.id, Some(t.b)).await.unwrap();
        forbidden(set(&t.tess, own.id, None).await);

        assert_eq!(
            t.audit_vias("course.unit").await,
            [
                (Some("unit".to_owned()), Some(t.b.to_string())),
                (Some("admin".to_owned()), None),
                (Some("admin".to_owned()), None),
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn list_unit_courses_covers_the_subtree() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let page = || PageParams::new(None, None).unwrap();
        let mut conn = t.conn().await;
        let ids = |page: Page<Course>| page.items.into_iter().map(|c| c.id).collect::<Vec<_>>();
        // K sits in B; A covers it, in every status, for the viewer and for an admin.
        for who in [&t.pat, &t.admin] {
            let listed = list_unit_courses(&mut conn, who, t.a, page())
                .await
                .unwrap();
            assert_eq!(ids(listed), [t.k]);
        }
        let listed = list_unit_courses(&mut conn, &t.max, t.b, page())
            .await
            .unwrap();
        assert_eq!(ids(listed), [t.k]);
        // A sibling holds nothing.
        let listed = list_unit_courses(&mut conn, &t.pat, t.s, page())
            .await
            .unwrap();
        assert!(listed.items.is_empty());
        // Without a grant the unit is not found, and so is a unit that does not exist.
        for (who, unit) in [(&t.tess, t.a), (&t.max, t.a), (&t.pat, Uuid::now_v7())] {
            let outcome = list_unit_courses(&mut conn, who, unit, page()).await;
            assert!(
                matches!(
                    outcome,
                    Err(TeachingError::Org(org::OrgError::UnitNotFound))
                ),
                "{outcome:?}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn can_tells_a_viewer_what_they_may_do() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let access = t.access(&t.pat).await;
        assert_eq!(
            access::course_can(&t.pat, access),
            access::CourseCan {
                view_grades: true,
                edit_content: false,
                grade: false,
                manage_roster: false,
                manage_course: false,
            }
        );
        let access = t.access(&t.max).await;
        let can = access::course_can(&t.max, access);
        assert!(can.view_grades && can.manage_roster && can.manage_course);
        assert!(!can.edit_content && !can.grade);
    })
    .await;
}

/// The grant is read after the lock is held, as the role is: a manager whose role was revoked by
/// the writer we waited for no longer moves anything.
#[tokio::test]
async fn a_manager_revoked_while_waiting_does_not_move_the_course() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let mut first = t.pool.begin().await.unwrap();
        // The admin holds the course lock (a rename) and revokes Max's role in the same
        // transaction.
        update_course(
            &mut first,
            &t.admin,
            t.k,
            CourseUpdate {
                title: Some("Held".to_owned()),
                term_id: None,
            },
        )
        .await
        .unwrap();
        org::remove_unit_role(&mut first, &t.admin, t.b, t.max.user_id())
            .await
            .unwrap();
        let second = tokio::spawn({
            let (pool, max, k) = (t.pool.clone(), t.max.clone(), t.k);
            async move {
                let mut tx = pool.begin().await.unwrap();
                set_course_unit(&mut tx, &max, k, None).await
            }
        });
        wait_until_blocked(&t.pool, &second).await;
        first.commit().await.unwrap();
        // With the role gone Max cannot even see the course.
        let outcome = second.await.unwrap();
        assert!(
            matches!(outcome, Err(TeachingError::CourseNotFound)),
            "{outcome:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn an_unknown_ability_in_the_database_is_an_error_not_a_smaller_grant() {
    let abilities = vec!["view_grades".to_owned(), "fly".to_owned()];
    let outcome = unit_grant_of(Uuid::now_v7(), abilities);
    assert!(
        matches!(outcome, Err(TeachingError::Db(sqlx::Error::Decode(_)))),
        "{outcome:?}"
    );
}

/// A manager of S who studies in a course is a student there: they may not put it under S, which
/// would give them a manager's view of it. Not even a move with no unit on either side goes
/// through for a student of the course.
#[tokio::test]
async fn a_student_of_a_course_does_not_pull_it_under_their_unit() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let hash = auth::hash_password("irrelevant").await.unwrap();
        let sam = actor(&t.pool, &hash, "sam@x.edu", Role::Teacher).await;
        let mut tx = t.pool.begin().await.unwrap();
        org::set_unit_role(
            &mut tx,
            &t.admin,
            t.s,
            sam.user_id(),
            access::UnitRole::Manager,
        )
        .await
        .unwrap();
        let course = create_course(
            &mut tx,
            &t.admin,
            CourseTitle::parse("Studied").unwrap(),
            None,
            None,
        )
        .await
        .unwrap();
        put_member(
            &mut tx,
            &t.admin,
            course.id,
            sam.user_id(),
            CourseRole::Student,
        )
        .await
        .unwrap();
        publish_course(&mut tx, &t.admin, course.id).await.unwrap();
        tx.commit().await.unwrap();

        for unit in [Some(t.s), None] {
            let mut tx = t.pool.begin().await.unwrap();
            let outcome = set_course_unit(&mut tx, &sam, course.id, unit).await;
            assert!(
                matches!(outcome, Err(TeachingError::Forbidden(_))),
                "{unit:?}: {outcome:?}"
            );
        }
    })
    .await;
}
