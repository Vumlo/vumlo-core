use crate::tests::db_test;
use test_support::wait_until_blocked;

use sqlx::PgPool;

use super::*;
use access::UnitAbility;
use auth::{self, Email, NewUser, PersonName};
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

// ---------------------------------------------------------------------------------------------
// Units (ADR-0094)
// ---------------------------------------------------------------------------------------------

/// Pat, a teacher who is `manager` of A; A > B; S is a top-level unit Pat holds nothing on.
struct Tree {
    pool: PgPool,
    admin: Actor,
    pat: Actor,
    a: Uuid,
    b: Uuid,
    s: Uuid,
}

fn actor(id: Uuid, role: Role) -> Actor {
    Actor::for_tests(id, role, Uuid::now_v7())
}

fn name(raw: &str) -> OrgName {
    OrgName::parse(raw).unwrap()
}

async fn tree(pool: PgPool) -> Tree {
    let admin = actor(user(&pool, "admin@x.edu", Role::Admin).await, Role::Admin);
    let pat_id = user(&pool, "pat@x.edu", Role::Teacher).await;
    let mut tx = pool.begin().await.unwrap();
    let a = create_unit(&mut tx, &admin, name("A"), None).await.unwrap();
    let b = create_unit(&mut tx, &admin, name("B"), Some(a.id))
        .await
        .unwrap();
    let s = create_unit(&mut tx, &admin, name("S"), None).await.unwrap();
    sqlx::query!(
        "INSERT INTO org.unit_roles (unit_id, user_id, role) VALUES ($1, $2, 'manager')",
        a.id,
        pat_id
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    Tree {
        pool,
        admin,
        pat: actor(pat_id, Role::Teacher),
        a: a.id,
        b: b.id,
        s: s.id,
    }
}

impl Tree {
    async fn conn(&self) -> sqlx::pool::PoolConnection<sqlx::Postgres> {
        self.pool.acquire().await.unwrap()
    }

    async fn unit(&self, id: Uuid) -> Option<(Option<Uuid>, String)> {
        sqlx::query!("SELECT parent_id, name FROM org.units WHERE id = $1", id)
            .fetch_optional(&self.pool)
            .await
            .unwrap()
            .map(|row| (row.parent_id, row.name))
    }

    async fn audited(&self, action: &str) -> i64 {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM events.audit_log WHERE action = $1"#,
            action
        )
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }
}

#[tokio::test]
async fn a_manager_creates_renames_moves_and_deletes_below_their_unit() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let mut tx = t.pool.begin().await.unwrap();

        let physics = create_unit(&mut tx, &t.pat, name("Physics"), Some(t.a))
            .await
            .unwrap();
        assert_eq!(physics.parent_id, Some(t.a));
        let renamed = update_unit(
            &mut tx,
            &t.pat,
            t.b,
            UnitUpdate {
                name: Some(name("Biology")),
                parent_id: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(renamed.name, "Biology");
        let c = create_unit(&mut tx, &t.pat, name("C"), Some(t.b))
            .await
            .unwrap();
        let moved = update_unit(
            &mut tx,
            &t.pat,
            c.id,
            UnitUpdate {
                name: None,
                parent_id: Some(Some(t.a)),
            },
        )
        .await
        .unwrap();
        assert_eq!(moved.parent_id, Some(t.a));
        delete_unit(&mut tx, &t.pat, c.id).await.unwrap();
        tx.commit().await.unwrap();

        assert!(t.unit(c.id).await.is_none());
        assert_eq!(t.audited("unit.create").await, 3 + 2);
        assert_eq!(t.audited("unit.rename").await, 1);
        assert_eq!(t.audited("unit.move").await, 1);
        assert_eq!(t.audited("unit.delete").await, 1);
    })
    .await;
}

#[tokio::test]
async fn a_manager_cannot_touch_their_own_unit_or_create_at_the_top() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let mut conn = t.pool.begin().await.unwrap(); // a move takes a table lock

        let rename = UnitUpdate {
            name: Some(name("A2")),
            parent_id: None,
        };
        let to_top = UnitUpdate {
            name: None,
            parent_id: Some(None),
        };
        fn forbidden<T>(outcome: &Result<T, OrgError>) -> bool {
            matches!(outcome, Err(OrgError::Forbidden(_)))
        }

        assert!(forbidden(
            &update_unit(&mut conn, &t.pat, t.a, rename.clone()).await
        ));
        assert!(forbidden(
            &update_unit(&mut conn, &t.pat, t.a, to_top).await
        ));
        assert!(forbidden(&delete_unit(&mut conn, &t.pat, t.a).await));
        assert!(forbidden(
            &create_unit(&mut conn, &t.pat, name("Top"), None).await
        ));
        // A unit Pat holds nothing on does not exist for them.
        assert!(matches!(
            update_unit(&mut conn, &t.pat, t.s, rename.clone()).await,
            Err(OrgError::UnitNotFound)
        ));
        assert!(matches!(
            delete_unit(&mut conn, &t.pat, t.s).await,
            Err(OrgError::UnitNotFound)
        ));
        assert!(matches!(
            create_unit(&mut conn, &t.pat, name("X"), Some(t.s)).await,
            Err(OrgError::UnitNotFound)
        ));
        // Nor can they move a unit of theirs under one they do not see.
        assert!(matches!(
            update_unit(
                &mut conn,
                &t.pat,
                t.b,
                UnitUpdate {
                    name: None,
                    parent_id: Some(Some(t.s))
                }
            )
            .await,
            Err(OrgError::UnitNotFound)
        ));

        // The administrator can do all of it.
        create_unit(&mut conn, &t.admin, name("Top"), None)
            .await
            .unwrap();
        update_unit(&mut conn, &t.admin, t.a, rename).await.unwrap();
        conn.commit().await.unwrap();
        assert_eq!(t.unit(t.a).await.unwrap().1, "A2");
    })
    .await;
}

#[tokio::test]
async fn a_unit_cannot_move_under_itself_or_below() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let mut conn = t.pool.begin().await.unwrap(); // a move takes a table lock

        for new_parent in [t.b, t.a] {
            let outcome = update_unit(
                &mut conn,
                &t.admin,
                t.a,
                UnitUpdate {
                    name: None,
                    parent_id: Some(Some(new_parent)),
                },
            )
            .await;
            assert!(matches!(outcome, Err(OrgError::UnitCycle)), "{outcome:?}");
        }
        assert_eq!(t.unit(t.a).await.unwrap().0, None);
    })
    .await;
}

#[tokio::test]
async fn a_unit_with_subunits_is_not_deleted() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let mut conn = t.conn().await;

        let outcome = delete_unit(&mut conn, &t.admin, t.a).await;
        assert!(matches!(outcome, Err(OrgError::UnitInUse)), "{outcome:?}");

        delete_unit(&mut conn, &t.admin, t.b).await.unwrap();
        delete_unit(&mut conn, &t.admin, t.a).await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn sibling_names_are_unique_top_level_too() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let mut conn = t.conn().await;

        create_unit(&mut conn, &t.admin, name("Science"), None)
            .await
            .unwrap();
        let again = create_unit(&mut conn, &t.admin, name("Science"), None).await;
        assert!(matches!(again, Err(OrgError::UnitNameTaken)), "{again:?}");
        for parent in [t.a, t.b] {
            create_unit(&mut conn, &t.admin, name("Science"), Some(parent))
                .await
                .unwrap();
        }
        // A rename and a move into a sibling's name are refused alike.
        let rename = update_unit(
            &mut conn,
            &t.admin,
            t.s,
            UnitUpdate {
                name: Some(name("Science")),
                parent_id: None,
            },
        )
        .await;
        assert!(matches!(rename, Err(OrgError::UnitNameTaken)), "{rename:?}");
    })
    .await;
}

#[tokio::test]
async fn list_units_shows_the_covered_subtree_with_abilities() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let student = actor(
            user(&t.pool, "kim@x.edu", Role::Student).await,
            Role::Student,
        );
        let mut conn = t.conn().await;
        let manager = [
            UnitAbility::ViewGrades,
            UnitAbility::ManageCourses,
            UnitAbility::ManageRoster,
            UnitAbility::ManageUnits,
        ];

        let mut mine = list_units(&mut conn, &t.pat).await.unwrap();
        mine.sort_by_key(|(unit, _)| unit.name.clone());
        let names: Vec<_> = mine.iter().map(|(unit, _)| unit.name.as_str()).collect();
        assert_eq!(names, ["A", "B"]);
        for (unit, grant) in &mine {
            assert_eq!(*grant, UnitGrant::new(unit.id, manager));
        }

        let all = list_units(&mut conn, &t.admin).await.unwrap();
        assert_eq!(all.len(), 3);
        for (unit, grant) in &all {
            assert_eq!(*grant, UnitGrant::full(unit.id));
        }

        assert!(list_units(&mut conn, &student).await.unwrap().is_empty());

        assert_eq!(
            unit_grant(&mut conn, &t.pat, t.b).await.unwrap(),
            Some(UnitGrant::new(t.b, manager))
        );
        assert_eq!(unit_grant(&mut conn, &t.pat, t.s).await.unwrap(), None);
        assert!(matches!(
            unit_grant(&mut conn, &t.pat, Uuid::now_v7()).await,
            Err(OrgError::UnitNotFound)
        ));
    })
    .await;
}
