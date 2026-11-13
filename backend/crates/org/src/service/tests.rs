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

/// A move that the caller may not make is refused before the tree lock is asked for: holding the
/// lock in another transaction must not stall it (otherwise any signed-in user could stall every
/// writer of the tree by sending moves).
#[tokio::test]
async fn an_unauthorized_move_is_refused_without_taking_the_tree_lock() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let kim = actor(
            user(&t.pool, "kim@x.edu", Role::Student).await,
            Role::Student,
        );
        let mut holder = t.pool.begin().await.unwrap();
        repo::lock_tree(&mut holder).await.unwrap();

        // (who, unit, new parent, whether the refusal is "forbidden" rather than "not found")
        let attempts = [
            (t.pat.clone(), t.a, None, true),
            (t.pat.clone(), t.s, Some(t.a), false),
            (t.pat.clone(), t.b, Some(t.s), false),
            (t.pat.clone(), Uuid::now_v7(), Some(t.a), false),
            (kim.clone(), t.b, Some(t.a), false),
        ];
        for (who, unit, parent, forbidden) in attempts {
            let pool = t.pool.clone();
            let attempt = tokio::spawn(async move {
                let mut tx = pool.begin().await.unwrap();
                let update = UnitUpdate {
                    name: None,
                    parent_id: Some(parent),
                };
                update_unit(&mut tx, &who, unit, update).await
            });
            let outcome = tokio::time::timeout(std::time::Duration::from_secs(10), attempt)
                .await
                .expect("the refused move waited for the tree lock")
                .unwrap();
            assert!(
                match &outcome {
                    Err(OrgError::Forbidden(_)) => forbidden,
                    Err(OrgError::UnitNotFound) => !forbidden,
                    _ => false,
                },
                "{outcome:?}"
            );
        }
        holder.rollback().await.unwrap();
    })
    .await;
}

/// A rename writes the name only. The unit's parent was changed by a committed statement after the
/// caller's read (which a service call cannot be made to interleave deterministically); the rename
/// must leave the new parent alone, where the old whole-row update would have written the stale one.
#[tokio::test]
async fn a_rename_leaves_the_parent_alone() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let mut conn = t.conn().await;
        sqlx::query!(
            "UPDATE org.units SET parent_id = $2 WHERE id = $1",
            t.b,
            t.s
        )
        .execute(&mut *conn)
        .await
        .unwrap();

        let renamed = repo::rename_unit(&mut conn, t.b, "Bio")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(renamed.parent_id, Some(t.s));
        assert_eq!(t.unit(t.b).await, Some((Some(t.s), "Bio".to_owned())));
        assert!(
            repo::rename_unit(&mut conn, Uuid::now_v7(), "x")
                .await
                .unwrap()
                .is_none()
        );
    })
    .await;
}

/// Two moves that are each fine alone close a cycle together: the second one passed its unlocked
/// check, waited for the tree lock, and must see the first one's result when it checks again.
#[tokio::test]
async fn a_move_checks_again_under_the_tree_lock() {
    db_test(async |pool| {
        let t = tree(pool).await;
        // S under B is fine now (A > B, S top-level).
        let mut first = t.pool.begin().await.unwrap();
        repo::lock_tree(&mut first).await.unwrap();
        let second = tokio::spawn({
            let (pool, admin, s, b) = (t.pool.clone(), t.admin.clone(), t.s, t.b);
            async move {
                let mut tx = pool.begin().await.unwrap();
                let update = UnitUpdate {
                    name: None,
                    parent_id: Some(Some(b)),
                };
                update_unit(&mut tx, &admin, s, update).await
            }
        });
        wait_until_blocked(&t.pool, &second).await;
        // Meanwhile B goes under S, which makes S under B a cycle.
        repo::move_unit(&mut first, t.b, Some(t.s), None)
            .await
            .unwrap();
        first.commit().await.unwrap();

        let outcome = second.await.unwrap();
        assert!(matches!(outcome, Err(OrgError::UnitCycle)), "{outcome:?}");
        assert_eq!(t.unit(t.s).await.unwrap().0, None);
        assert_eq!(t.unit(t.b).await.unwrap().0, Some(t.s));
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// Unit roles and the abilities admins switch (ADR-0094)
// ---------------------------------------------------------------------------------------------

async fn person(t: &Tree, email: &str, role: Role) -> (Uuid, Actor) {
    let id = user(&t.pool, email, role).await;
    (id, actor(id, role))
}

impl Tree {
    async fn audited_in(&self, conn: &mut sqlx::PgConnection, action: &str) -> i64 {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM events.audit_log WHERE action = $1"#,
            action
        )
        .fetch_one(conn)
        .await
        .unwrap()
    }

    async fn roles_of(&self, user_id: Uuid) -> Vec<(Uuid, String)> {
        sqlx::query!(
            "SELECT unit_id, role FROM org.unit_roles WHERE user_id = $1 ORDER BY unit_id",
            user_id
        )
        .fetch_all(&self.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.unit_id, row.role))
        .collect()
    }
}

#[tokio::test]
async fn a_student_cannot_hold_a_unit_role() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let (kim, _) = person(&t, "kim@x.edu", Role::Student).await;
        let mut tx = t.pool.begin().await.unwrap();

        let outcome = set_unit_role(&mut tx, &t.admin, t.b, kim, UnitRole::Viewer).await;
        assert!(
            matches!(outcome, Err(OrgError::RoleNotAllowed)),
            "{outcome:?}"
        );
        let outcome = set_unit_role(&mut tx, &t.admin, t.b, Uuid::now_v7(), UnitRole::Viewer).await;
        assert!(
            matches!(outcome, Err(OrgError::MemberNotFound)),
            "{outcome:?}"
        );
        tx.commit().await.unwrap();

        assert!(t.roles_of(kim).await.is_empty());
        assert_eq!(t.audited("unit.role.set").await, 0);
    })
    .await;
}

#[tokio::test]
async fn a_manager_grants_below_but_not_on_their_own_unit() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let (quinn, quinn_actor) = person(&t, "quinn@x.edu", Role::Teacher).await;
        let mut tx = t.pool.begin().await.unwrap();

        set_unit_role(&mut tx, &t.pat, t.b, quinn, UnitRole::Viewer)
            .await
            .unwrap();
        // Idempotent: the same grant again changes nothing and leaves no second audit row.
        set_unit_role(&mut tx, &t.pat, t.b, quinn, UnitRole::Viewer)
            .await
            .unwrap();
        assert_eq!(t.audited_in(&mut tx, "unit.role.set").await, 1);
        let listed = list_unit_roles(&mut tx, &t.pat, t.b).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            (listed[0].user_id, listed[0].role, listed[0].email.as_str()),
            (quinn, UnitRole::Viewer, "quinn@x.edu")
        );
        // A role changes in place.
        set_unit_role(&mut tx, &t.pat, t.b, quinn, UnitRole::Manager)
            .await
            .unwrap();
        assert_eq!(
            list_unit_roles(&mut tx, &t.pat, t.b).await.unwrap()[0].role,
            UnitRole::Manager
        );

        // Not on the unit Pat's own role sits on, for anyone, themselves included.
        fn forbidden<T>(outcome: &Result<T, OrgError>) -> bool {
            matches!(outcome, Err(OrgError::Forbidden(_)))
        }
        assert!(forbidden(
            &set_unit_role(&mut tx, &t.pat, t.a, quinn, UnitRole::Viewer).await
        ));
        assert!(forbidden(
            &set_unit_role(&mut tx, &t.pat, t.a, t.pat.user_id(), UnitRole::Manager).await
        ));
        assert!(forbidden(
            &remove_unit_role(&mut tx, &t.pat, t.a, t.pat.user_id()).await
        ));
        // A unit Pat does not see does not exist for them.
        assert!(matches!(
            set_unit_role(&mut tx, &t.pat, t.s, quinn, UnitRole::Viewer).await,
            Err(OrgError::UnitNotFound)
        ));
        assert!(matches!(
            list_unit_roles(&mut tx, &t.pat, t.s).await,
            Err(OrgError::UnitNotFound)
        ));
        // Quinn manages B only, so nothing above it opens to them: they cannot grant on B.
        assert!(forbidden(
            &set_unit_role(
                &mut tx,
                &quinn_actor,
                t.b,
                t.pat.user_id(),
                UnitRole::Viewer
            )
            .await
        ));

        remove_unit_role(&mut tx, &t.pat, t.b, quinn).await.unwrap();
        assert!(matches!(
            remove_unit_role(&mut tx, &t.pat, t.b, quinn).await,
            Err(OrgError::MemberNotFound)
        ));
        tx.commit().await.unwrap();

        assert!(t.roles_of(quinn).await.is_empty());
        assert_eq!(t.audited("unit.role.set").await, 2);
        assert_eq!(t.audited("unit.role.remove").await, 1);
        // Pat's own role is untouched.
        assert_eq!(
            t.roles_of(t.pat.user_id()).await,
            [(t.a, "manager".to_owned())]
        );
    })
    .await;
}

#[tokio::test]
async fn switching_an_ability_off_changes_the_grant_at_once() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let (quinn, quinn_actor) = person(&t, "quinn@x.edu", Role::Teacher).await;
        let mut conn = t.pool.begin().await.unwrap();
        set_unit_role(&mut conn, &t.admin, t.b, quinn, UnitRole::Viewer)
            .await
            .unwrap();
        let before = unit_grant(&mut conn, &quinn_actor, t.b).await.unwrap();
        assert!(before.unwrap().has(UnitAbility::ViewGrades));

        set_unit_role_abilities(&mut conn, &t.admin, UnitRole::Viewer, vec![])
            .await
            .unwrap();
        let after = unit_grant(&mut conn, &quinn_actor, t.b).await.unwrap();
        let after = after.expect("a role with no switches still means view");
        assert!(!after.has(UnitAbility::ViewGrades));
        assert!(after.abilities().is_empty());

        // Replacing, not adding: the set comes back as sent, in order, without duplicates.
        set_unit_role_abilities(
            &mut conn,
            &t.admin,
            UnitRole::Viewer,
            vec![
                UnitAbility::Grade,
                UnitAbility::EditContent,
                UnitAbility::Grade,
            ],
        )
        .await
        .unwrap();
        let all = unit_role_abilities(&mut conn, &t.admin).await.unwrap();
        assert_eq!(
            all,
            [
                (
                    UnitRole::Manager,
                    vec![
                        UnitAbility::ViewGrades,
                        UnitAbility::ManageRoster,
                        UnitAbility::ManageCourses,
                        UnitAbility::ManageUnits,
                    ]
                ),
                (
                    UnitRole::Viewer,
                    vec![UnitAbility::EditContent, UnitAbility::Grade]
                ),
            ]
        );
        conn.commit().await.unwrap();
        assert_eq!(t.audited("unit.abilities.set").await, 2);
    })
    .await;
}

#[tokio::test]
async fn only_an_admin_sets_role_abilities() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let mut conn = t.conn().await;
        assert!(matches!(
            set_unit_role_abilities(&mut conn, &t.pat, UnitRole::Viewer, vec![]).await,
            Err(OrgError::Forbidden(_))
        ));
        assert!(matches!(
            unit_role_abilities(&mut conn, &t.pat).await,
            Err(OrgError::Forbidden(_))
        ));
        let viewer = sqlx::query_scalar!(
            "SELECT count(*) AS \"n!\" FROM org.unit_role_abilities WHERE role = 'viewer'"
        )
        .fetch_one(&t.pool)
        .await
        .unwrap();
        assert_eq!(viewer, 1);
        assert_eq!(t.audited("unit.abilities.set").await, 0);

        let mut tx = t.pool.begin().await.unwrap();
        set_unit_role_abilities(
            &mut tx,
            &t.admin,
            UnitRole::Viewer,
            vec![UnitAbility::Grade],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(t.audited("unit.abilities.set").await, 1);
    })
    .await;
}

#[tokio::test]
async fn candidates_are_active_teachers_and_admins_not_yet_on_the_unit() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let (lena, _) = person(&t, "lena@x.edu", Role::Teacher).await;
        let (ada, _) = person(&t, "ada-lena@x.edu", Role::Admin).await;
        let (_, _) = person(&t, "lena-kid@x.edu", Role::Student).await;
        let (held, _) = person(&t, "lena-held@x.edu", Role::Teacher).await;
        let mut tx = t.pool.begin().await.unwrap();
        set_unit_role(&mut tx, &t.admin, t.b, held, UnitRole::Viewer)
            .await
            .unwrap();

        let found = unit_candidates(&mut tx, &t.pat, t.b, "lena").await.unwrap();
        let mut ids: Vec<_> = found.iter().map(|c| c.user_id).collect();
        ids.sort();
        let mut expected = [lena, ada];
        expected.sort();
        assert_eq!(ids, expected, "{found:?}");
        // The percent sign is text, not a wildcard.
        assert!(
            unit_candidates(&mut tx, &t.pat, t.b, "l%a")
                .await
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            unit_candidates(&mut tx, &t.pat, t.b, " l ").await,
            Err(OrgError::QueryTooShort)
        ));
        // Whoever may not manage roles on the unit learns nothing about people.
        assert!(matches!(
            unit_candidates(&mut tx, &t.pat, t.a, "lena").await,
            Err(OrgError::Forbidden(_))
        ));
        assert!(matches!(
            unit_candidates(&mut tx, &t.pat, t.s, "lena").await,
            Err(OrgError::UnitNotFound)
        ));
    })
    .await;
}

/// A caller who may not grant on the unit must not be able to hold others up: the person's row is
/// locked only after the caller is authorized.
#[tokio::test]
async fn an_unauthorized_grant_does_not_wait_for_the_person() {
    db_test(async |pool| {
        let t = tree(pool).await;
        let (quinn, _) = person(&t, "quinn@x.edu", Role::Teacher).await;
        let mut holder = t.pool.begin().await.unwrap();
        sqlx::query!("SELECT id FROM auth.users WHERE id = $1 FOR UPDATE", quinn)
            .fetch_one(&mut *holder)
            .await
            .unwrap();

        let denied = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let mut conn = t.conn().await;
            (
                set_unit_role(&mut conn, &t.pat, t.s, quinn, UnitRole::Viewer).await,
                set_unit_role(&mut conn, &t.pat, t.a, quinn, UnitRole::Viewer).await,
            )
        })
        .await
        .expect("an unauthorized caller waited for a lock");
        assert!(matches!(denied.0, Err(OrgError::UnitNotFound)));
        assert!(matches!(denied.1, Err(OrgError::Forbidden(_))));
        holder.rollback().await.unwrap();
    })
    .await;
}

#[test]
fn an_unknown_ability_in_the_database_is_an_error_not_a_smaller_grant() {
    let abilities = vec!["view_grades".to_owned(), "fly".to_owned()];
    let outcome = grant_of(Uuid::now_v7(), &abilities);
    assert!(
        matches!(outcome, Err(OrgError::Db(sqlx::Error::Decode(_)))),
        "{outcome:?}"
    );
}
