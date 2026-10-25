use test_support::{db_test, wait_until_blocked};

use sqlx::PgPool;

use super::*;
use crate::auth::{self, Email, NewUser, PersonName};
use common::Role;

async fn user(pool: &PgPool, email: &str, role: Role) -> Uuid {
    let hash = auth::hash_password("irrelevant").await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    let user = auth::create_user_unaudited_for_tests(
        &mut tx,
        NewUser {
            email: Email::parse(email).unwrap(),
            name: PersonName::name("Test").unwrap(),
            surname: PersonName::surname("User").unwrap(),
            role,
        },
        hash,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    user.id
}

/// A group with one student in it, and an empty subgroup of the group.
struct Fixture {
    pool: PgPool,
    admin: Actor,
    group: Uuid,
    subgroup: Uuid,
    student: Uuid,
}

async fn fixture(pool: PgPool) -> Fixture {
    let admin = user(&pool, "admin@x.edu", Role::Admin).await;
    let student = user(&pool, "student@x.edu", Role::Student).await;
    let admin = Actor::for_tests(admin, Role::Admin, Uuid::now_v7());
    let mut tx = pool.begin().await.unwrap();
    let group = create_group(&mut tx, &admin, "7-A").await.unwrap();
    add_group_members(&mut tx, &admin, group.id, &[student])
        .await
        .unwrap();
    let subgroup = create_subgroup(&mut tx, &admin, group.id, "English 1")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    Fixture {
        pool,
        admin,
        group: group.id,
        subgroup: subgroup.id,
        student,
    }
}

impl Fixture {
    async fn group_members(&self) -> i64 {
        sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM org.group_members"#)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    async fn subgroup_members(&self) -> i64 {
        sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM org.subgroup_members"#)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
}

/// A removal from the group is in flight and holds the group lock. An add of the same user to
/// the subgroup must wait for it and then see that the user is no longer in the group. Without
/// the lock it would read the old membership, insert, and hit the foreign key (a 500).
#[tokio::test]
async fn an_add_to_a_subgroup_waits_for_a_removal_from_its_group_and_is_refused() {
    db_test(async |pool| {
        let f = fixture(pool).await;

        let mut first = f.pool.begin().await.unwrap();
        remove_group_member(&mut first, &f.admin, f.group, f.student)
            .await
            .unwrap();
        let second = tokio::spawn({
            let (pool, admin, subgroup, student) =
                (f.pool.clone(), f.admin.clone(), f.subgroup, f.student);
            async move {
                let mut tx = pool.begin().await.unwrap();
                add_subgroup_members(&mut tx, &admin, subgroup, &[student]).await
            }
        });
        wait_until_blocked(&f.pool, &second).await;
        first.commit().await.unwrap();

        let outcome = second.await.unwrap();
        assert!(
            matches!(&outcome, Err(OrgError::Members(refused))
                    if refused == &[(0, MemberProblem::NotInGroup)]),
            "{outcome:?}"
        );
        assert_eq!(f.group_members().await, 0);
        assert_eq!(f.subgroup_members().await, 0);
    })
    .await;
}

/// An add to the subgroup is in flight. A removal from the group must wait for it and then
/// take the user out of the subgroup too, leaving no subgroup row without a group row.
#[tokio::test]
async fn a_removal_from_a_group_waits_for_an_add_to_its_subgroup_and_takes_it_back() {
    db_test(async |pool| {
        let f = fixture(pool).await;

        let mut first = f.pool.begin().await.unwrap();
        let added = add_subgroup_members(&mut first, &f.admin, f.subgroup, &[f.student])
            .await
            .unwrap();
        assert_eq!(added, [f.student]);
        let second = tokio::spawn({
            let (pool, admin, group, student) =
                (f.pool.clone(), f.admin.clone(), f.group, f.student);
            async move {
                let mut tx = pool.begin().await.unwrap();
                remove_group_member(&mut tx, &admin, group, student).await?;
                tx.commit().await.map_err(OrgError::from)
            }
        });
        wait_until_blocked(&f.pool, &second).await;
        first.commit().await.unwrap();

        second.await.unwrap().unwrap();
        assert_eq!(f.group_members().await, 0);
        assert_eq!(f.subgroup_members().await, 0);
    })
    .await;
}
