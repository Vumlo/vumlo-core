use test_support::{db_test, wait_until_blocked};

use super::*;
use crate::auth::{self, Email, HashedPassword, NewUser, PersonName};
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
    Actor::new(user.id, role, Uuid::now_v7())
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
    let hash = auth::hash_password("irrelevant").await.unwrap();
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
