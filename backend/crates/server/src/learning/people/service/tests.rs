use super::*;
use crate::access::Actor;
use crate::auth::{self, AuthError, Email, NewUser, PersonName};
use crate::learning::teaching::{self, TeachingError};
use common::Role;
use test_support::{db_test, wait_until_blocked};
use uuid::Uuid;

async fn admin_actor(pool: &sqlx::PgPool, email: &str) -> Actor {
    let hash = auth::hash_password("irrelevant").await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    let user = auth::create_user_unaudited_for_tests(
        &mut tx,
        NewUser {
            email: Email::parse(email).unwrap(),
            name: PersonName::name("Admin").unwrap(),
            surname: PersonName::surname("Admin").unwrap(),
            role: Role::Admin,
        },
        hash,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    Actor::new(user.id, Role::Admin, Uuid::now_v7())
}

fn demote() -> UserUpdate {
    UserUpdate {
        role: Some(Role::Teacher),
        ..UserUpdate::default()
    }
}

#[tokio::test]
async fn concurrent_demotions_cannot_remove_every_admin() {
    db_test(async |pool| {
        let a = admin_actor(&pool, "a@x.edu").await;
        let b = admin_actor(&pool, "b@x.edu").await;

        // `a` demotes `b` and holds its transaction open; `b` tries to demote `a` meanwhile.
        let mut first = pool.begin().await.unwrap();
        update_user(&mut first, &a, b.user_id(), demote())
            .await
            .unwrap();
        let second = tokio::spawn({
            let pool = pool.clone();
            let (b, a_id) = (b.clone(), a.user_id());
            async move {
                let mut tx = pool.begin().await.unwrap();
                update_user(&mut tx, &b, a_id, demote()).await
            }
        });
        wait_until_blocked(&pool, &second).await;
        first.commit().await.unwrap();

        let outcome = second.await.unwrap();
        assert!(
            matches!(outcome, Err(LearningError::Auth(AuthError::LastAdmin))),
            "{outcome:?}"
        );
        let mut conn = pool.acquire().await.unwrap();
        let a_now = auth::find_user(&mut conn, a.user_id())
            .await
            .unwrap()
            .unwrap();
        let b_now = auth::find_user(&mut conn, b.user_id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!((a_now.role, b_now.role), (Role::Admin, Role::Teacher));
    })
    .await;
}

fn to_student() -> UserUpdate {
    UserUpdate {
        role: Some(Role::Student),
        ..UserUpdate::default()
    }
}

async fn teacher_actor(pool: &sqlx::PgPool, email: &str) -> Actor {
    let hash = auth::hash_password("irrelevant").await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    let user = auth::create_user_unaudited_for_tests(
        &mut tx,
        NewUser {
            email: Email::parse(email).unwrap(),
            name: PersonName::name("Teacher").unwrap(),
            surname: PersonName::surname("Teacher").unwrap(),
            role: Role::Teacher,
        },
        hash,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    Actor::new(user.id, Role::Teacher, Uuid::now_v7())
}

async fn course_of(pool: &sqlx::PgPool, owner: &Actor) -> Uuid {
    let mut tx = pool.begin().await.unwrap();
    let course = teaching::create_course(
        &mut tx,
        owner,
        teaching::CourseTitle::parse("Algebra").unwrap(),
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    course.id
}

/// A member write holds the user's row `FOR SHARE` from the moment it reads the role; the
/// demotion that comes meanwhile waits, and then removes the membership the write added.
#[tokio::test]
async fn a_demotion_waits_for_a_member_write_and_then_removes_the_membership() {
    db_test(async |pool| {
        let admin = admin_actor(&pool, "a@x.edu").await;
        let owner = teacher_actor(&pool, "owner@x.edu").await;
        let candidate = teacher_actor(&pool, "candidate@x.edu").await;
        let course = course_of(&pool, &owner).await;

        let mut first = pool.begin().await.unwrap();
        teaching::put_member(
            &mut first,
            &owner,
            course,
            candidate.user_id(),
            crate::access::CourseRole::Teacher,
        )
        .await
        .unwrap();
        let second = tokio::spawn({
            let pool = pool.clone();
            let (admin, who) = (admin.clone(), candidate.user_id());
            async move {
                let mut tx = pool.begin().await.unwrap();
                update_user(&mut tx, &admin, who, to_student()).await?;
                tx.commit().await.map_err(LearningError::from)
            }
        });
        wait_until_blocked(&pool, &second).await;
        first.commit().await.unwrap();
        second.await.unwrap().unwrap();

        // The course is a draft: only a member who is staff can still see it.
        let access = teaching::course_access(
            &mut pool.acquire().await.unwrap(),
            &Actor::new(candidate.user_id(), Role::Student, Uuid::now_v7()),
            course,
        )
        .await;
        assert!(
            matches!(access, Err(TeachingError::CourseNotFound)),
            "no teacher membership survives the demotion: {access:?}"
        );
    })
    .await;
}

/// The other order: the demotion is in flight, and a member write that comes meanwhile waits
/// for it and then reads the new role.
#[tokio::test]
async fn a_member_write_waits_for_a_demotion_and_then_refuses_the_teacher_role() {
    db_test(async |pool| {
        let admin = admin_actor(&pool, "a@x.edu").await;
        let owner = teacher_actor(&pool, "owner@x.edu").await;
        let candidate = teacher_actor(&pool, "candidate@x.edu").await;
        let course = course_of(&pool, &owner).await;

        let mut first = pool.begin().await.unwrap();
        update_user(&mut first, &admin, candidate.user_id(), to_student())
            .await
            .unwrap();
        let second = tokio::spawn({
            let pool = pool.clone();
            let (owner, who) = (owner.clone(), candidate.user_id());
            async move {
                let mut tx = pool.begin().await.unwrap();
                teaching::put_member(
                    &mut tx,
                    &owner,
                    course,
                    who,
                    crate::access::CourseRole::Teacher,
                )
                .await
            }
        });
        wait_until_blocked(&pool, &second).await;
        first.commit().await.unwrap();

        let outcome = second.await.unwrap();
        assert!(
            matches!(outcome, Err(TeachingError::RoleNotAllowed)),
            "{outcome:?}"
        );
    })
    .await;
}
