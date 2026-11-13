//! SQL only. Every function takes the connection it runs on and never begins a transaction.

use chrono::NaiveDate;
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{Group, GroupMember, MemberLine, Subgroup, Term, Unit};

// ---------------------------------------------------------------------------------------------
// Terms
// ---------------------------------------------------------------------------------------------

pub async fn insert_term(
    conn: &mut PgConnection,
    id: Uuid,
    name: &str,
    starts_on: NaiveDate,
    ends_on: NaiveDate,
) -> Result<Term, sqlx::Error> {
    sqlx::query_as!(
        Term,
        "INSERT INTO org.terms (id, name, starts_on, ends_on) VALUES ($1, $2, $3, $4)
         RETURNING id, name, starts_on, ends_on",
        id,
        name,
        starts_on,
        ends_on
    )
    .fetch_one(conn)
    .await
}

pub async fn find_term(conn: &mut PgConnection, id: Uuid) -> Result<Option<Term>, sqlx::Error> {
    sqlx::query_as!(
        Term,
        "SELECT id, name, starts_on, ends_on FROM org.terms WHERE id = $1",
        id
    )
    .fetch_optional(conn)
    .await
}

/// Locks the term row until the transaction ends, so an update merges into the committed row.
pub async fn lock_term(conn: &mut PgConnection, id: Uuid) -> Result<Option<Term>, sqlx::Error> {
    sqlx::query_as!(
        Term,
        "SELECT id, name, starts_on, ends_on FROM org.terms WHERE id = $1 FOR UPDATE",
        id
    )
    .fetch_optional(conn)
    .await
}

pub async fn list_terms(
    conn: &mut PgConnection,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Term>, sqlx::Error> {
    sqlx::query_as!(
        Term,
        "SELECT id, name, starts_on, ends_on FROM org.terms
         WHERE ($1::uuid IS NULL OR id > $1)
         ORDER BY id LIMIT $2",
        after,
        limit
    )
    .fetch_all(conn)
    .await
}

pub async fn update_term(
    conn: &mut PgConnection,
    id: Uuid,
    name: &str,
    starts_on: NaiveDate,
    ends_on: NaiveDate,
) -> Result<Term, sqlx::Error> {
    sqlx::query_as!(
        Term,
        "UPDATE org.terms SET name = $2, starts_on = $3, ends_on = $4 WHERE id = $1
         RETURNING id, name, starts_on, ends_on",
        id,
        name,
        starts_on,
        ends_on
    )
    .fetch_one(conn)
    .await
}

pub async fn delete_term(conn: &mut PgConnection, id: Uuid) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM org.terms WHERE id = $1", id)
        .execute(conn)
        .await?;
    Ok(result.rows_affected())
}

// ---------------------------------------------------------------------------------------------
// Groups
// ---------------------------------------------------------------------------------------------

pub async fn insert_group(
    conn: &mut PgConnection,
    id: Uuid,
    name: &str,
) -> Result<Group, sqlx::Error> {
    sqlx::query_as!(
        Group,
        "INSERT INTO org.groups (id, name) VALUES ($1, $2) RETURNING id, name, created_at",
        id,
        name
    )
    .fetch_one(conn)
    .await
}

pub async fn find_group(conn: &mut PgConnection, id: Uuid) -> Result<Option<Group>, sqlx::Error> {
    sqlx::query_as!(
        Group,
        "SELECT id, name, created_at FROM org.groups WHERE id = $1",
        id
    )
    .fetch_optional(conn)
    .await
}

/// Locks the group row until the transaction ends. Every write below a group queues here
/// (ADR-0032). It returns the group only: a statement that waits for a row lock re-reads just the
/// locked row, so the caller reads memberships in a later statement, once it holds the lock.
pub async fn lock_group(conn: &mut PgConnection, id: Uuid) -> Result<Option<Group>, sqlx::Error> {
    sqlx::query_as!(
        Group,
        "SELECT id, name, created_at FROM org.groups WHERE id = $1 FOR UPDATE",
        id
    )
    .fetch_optional(conn)
    .await
}

pub async fn list_groups(
    conn: &mut PgConnection,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Group>, sqlx::Error> {
    sqlx::query_as!(
        Group,
        "SELECT id, name, created_at FROM org.groups
         WHERE ($1::uuid IS NULL OR id > $1)
         ORDER BY id LIMIT $2",
        after,
        limit
    )
    .fetch_all(conn)
    .await
}

pub async fn rename_group(
    conn: &mut PgConnection,
    id: Uuid,
    name: &str,
) -> Result<Option<Group>, sqlx::Error> {
    sqlx::query_as!(
        Group,
        "UPDATE org.groups SET name = $2 WHERE id = $1 RETURNING id, name, created_at",
        id,
        name
    )
    .fetch_optional(conn)
    .await
}

/// Deletes the group; its subgroups and members go with it.
pub async fn delete_group(conn: &mut PgConnection, id: Uuid) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM org.groups WHERE id = $1", id)
        .execute(conn)
        .await?;
    Ok(result.rows_affected())
}

// ---------------------------------------------------------------------------------------------
// Units
// ---------------------------------------------------------------------------------------------

pub async fn insert_unit(
    conn: &mut PgConnection,
    id: Uuid,
    parent_id: Option<Uuid>,
    name: &str,
) -> Result<Unit, sqlx::Error> {
    sqlx::query_as!(
        Unit,
        "INSERT INTO org.units (id, parent_id, name) VALUES ($1, $2, $3)
         RETURNING id, parent_id, name, created_at",
        id,
        parent_id,
        name
    )
    .fetch_one(conn)
    .await
}

pub async fn find_unit(conn: &mut PgConnection, id: Uuid) -> Result<Option<Unit>, sqlx::Error> {
    sqlx::query_as!(
        Unit,
        "SELECT id, parent_id, name, created_at FROM org.units WHERE id = $1",
        id
    )
    .fetch_optional(conn)
    .await
}

/// Blocks every other writer of the tree until the transaction ends (it still lets readers in), so
/// a move checks its new parent against a tree no one else is changing. Two concurrent moves
/// could each look fine alone and close a cycle together.
pub async fn lock_tree(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query!("LOCK TABLE org.units IN SHARE ROW EXCLUSIVE MODE")
        .execute(conn)
        .await?;
    Ok(())
}

/// Is `unit_id` the root itself or a unit below it?
pub async fn in_subtree(
    conn: &mut PgConnection,
    root_id: Uuid,
    unit_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM org.unit_subtree WHERE root_id = $1 AND unit_id = $2
           ) AS "found!""#,
        root_id,
        unit_id
    )
    .fetch_one(conn)
    .await
}

/// The abilities (as text) of a person's grant on a unit, or `None` when they hold none there.
pub async fn unit_abilities(
    conn: &mut PgConnection,
    user_id: Uuid,
    unit_id: Uuid,
) -> Result<Option<Vec<String>>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT abilities AS "abilities!" FROM org.unit_grants WHERE user_id = $1 AND unit_id = $2"#,
        user_id,
        unit_id
    )
    .fetch_optional(conn)
    .await
}

/// Every unit, by name.
pub async fn list_units(conn: &mut PgConnection) -> Result<Vec<Unit>, sqlx::Error> {
    sqlx::query_as!(
        Unit,
        "SELECT id, parent_id, name, created_at FROM org.units ORDER BY name, id"
    )
    .fetch_all(conn)
    .await
}

/// The units a person's roles cover, with the abilities of the grant, by name.
pub async fn list_units_of_user(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<Vec<(Unit, Vec<String>)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT u.id, u.parent_id, u.name, u.created_at, g.abilities AS "abilities!"
           FROM org.unit_grants g
           JOIN org.units u ON u.id = g.unit_id
           WHERE g.user_id = $1
           ORDER BY u.name, u.id"#,
        user_id
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                Unit {
                    id: row.id,
                    parent_id: row.parent_id,
                    name: row.name,
                    created_at: row.created_at,
                },
                row.abilities,
            )
        })
        .collect())
}

pub async fn update_unit(
    conn: &mut PgConnection,
    id: Uuid,
    parent_id: Option<Uuid>,
    name: &str,
) -> Result<Unit, sqlx::Error> {
    sqlx::query_as!(
        Unit,
        "UPDATE org.units SET parent_id = $2, name = $3 WHERE id = $1
         RETURNING id, parent_id, name, created_at",
        id,
        parent_id,
        name
    )
    .fetch_one(conn)
    .await
}

/// Deletes the unit; its role rows go with it. Subunits and courses refuse (no cascade).
pub async fn delete_unit(conn: &mut PgConnection, id: Uuid) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM org.units WHERE id = $1", id)
        .execute(conn)
        .await?;
    Ok(result.rows_affected())
}

// ---------------------------------------------------------------------------------------------
// Group members
// ---------------------------------------------------------------------------------------------

/// The groups a person is in, by name.
pub async fn groups_of_user(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<Vec<(Uuid, String)>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT g.id, g.name FROM org.group_members m
         JOIN org.groups g ON g.id = m.group_id
         WHERE m.user_id = $1 ORDER BY g.name, g.id",
        user_id
    )
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(|row| (row.id, row.name)).collect())
}

/// The members of a group with their names and emails, by surname.
pub async fn group_member_lines(
    conn: &mut PgConnection,
    group_id: Uuid,
) -> Result<Vec<MemberLine>, sqlx::Error> {
    sqlx::query_as!(
        MemberLine,
        r#"SELECT u.id AS "user_id!", u.name AS "name!", u.surname AS "surname!", u.email AS "email!",
                m.added_at
         FROM org.group_members m
         JOIN auth.people u ON u.id = m.user_id
         WHERE m.group_id = $1
         ORDER BY u.surname, u.name, u.id"#,
        group_id
    )
    .fetch_all(conn)
    .await
}

/// Every term, newest first.
pub async fn all_terms_newest_first(conn: &mut PgConnection) -> Result<Vec<Term>, sqlx::Error> {
    sqlx::query_as!(
        Term,
        "SELECT id, name, starts_on, ends_on FROM org.terms ORDER BY starts_on DESC, id"
    )
    .fetch_all(conn)
    .await
}

/// Which of `user_ids` are members of the group.
pub async fn group_members_among(
    conn: &mut PgConnection,
    group_id: Uuid,
    user_ids: &[Uuid],
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT user_id FROM org.group_members WHERE group_id = $1 AND user_id = ANY($2)",
        group_id,
        user_ids
    )
    .fetch_all(conn)
    .await
}

/// Every member of the group.
pub async fn group_member_ids(
    conn: &mut PgConnection,
    group_id: Uuid,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT user_id FROM org.group_members WHERE group_id = $1",
        group_id
    )
    .fetch_all(conn)
    .await
}

/// Removes the members among `user_ids`, with their rows in the group's subgroups (the composite
/// key), and returns them.
pub async fn delete_group_members(
    conn: &mut PgConnection,
    group_id: Uuid,
    user_ids: &[Uuid],
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "DELETE FROM org.group_members WHERE group_id = $1 AND user_id = ANY($2)
         RETURNING user_id",
        group_id,
        user_ids
    )
    .fetch_all(conn)
    .await
}

/// Adds the users who are not members yet and returns them.
pub async fn insert_group_members(
    conn: &mut PgConnection,
    group_id: Uuid,
    user_ids: &[Uuid],
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"INSERT INTO org.group_members (group_id, user_id)
           SELECT $1, u FROM UNNEST($2::uuid[]) AS u
           ON CONFLICT DO NOTHING
           RETURNING user_id"#,
        group_id,
        user_ids
    )
    .fetch_all(conn)
    .await
}

/// Removes the member; their rows in the group's subgroups go with it (the composite key).
pub async fn delete_group_member(
    conn: &mut PgConnection,
    group_id: Uuid,
    user_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM org.group_members WHERE group_id = $1 AND user_id = $2",
        group_id,
        user_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// The members of the group after `after` (a user id), by surname, name and id, at most `limit`.
/// The place of the cursor in the order is read from `auth.people` (a read-only JOIN, as the
/// gradebook roster does), so it also works for a user who has just left.
pub async fn list_group_members(
    conn: &mut PgConnection,
    group_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<GroupMember>, sqlx::Error> {
    sqlx::query_as!(
        GroupMember,
        r#"SELECT u.id AS "user_id!", u.name AS "name!", u.surname AS "surname!"
           FROM org.group_members m
           JOIN auth.people u ON u.id = m.user_id
           WHERE m.group_id = $1
             AND ($2::uuid IS NULL
                  OR (u.surname, u.name, u.id) > (SELECT c.surname, c.name, c.id
                                                  FROM auth.people c WHERE c.id = $2))
           ORDER BY u.surname, u.name, u.id
           LIMIT $3"#,
        group_id,
        after,
        limit
    )
    .fetch_all(conn)
    .await
}

// ---------------------------------------------------------------------------------------------
// Subgroups
// ---------------------------------------------------------------------------------------------

pub async fn insert_subgroup(
    conn: &mut PgConnection,
    id: Uuid,
    group_id: Uuid,
    name: &str,
) -> Result<Subgroup, sqlx::Error> {
    sqlx::query_as!(
        Subgroup,
        "INSERT INTO org.subgroups (id, group_id, name) VALUES ($1, $2, $3)
         RETURNING id, group_id, name",
        id,
        group_id,
        name
    )
    .fetch_one(conn)
    .await
}

pub async fn find_subgroup(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<Subgroup>, sqlx::Error> {
    sqlx::query_as!(
        Subgroup,
        "SELECT id, group_id, name FROM org.subgroups WHERE id = $1",
        id
    )
    .fetch_optional(conn)
    .await
}

pub async fn list_subgroups(
    conn: &mut PgConnection,
    group_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Subgroup>, sqlx::Error> {
    sqlx::query_as!(
        Subgroup,
        "SELECT id, group_id, name FROM org.subgroups
         WHERE group_id = $1 AND ($2::uuid IS NULL OR id > $2)
         ORDER BY id LIMIT $3",
        group_id,
        after,
        limit
    )
    .fetch_all(conn)
    .await
}

/// The subgroups of these groups, in id order.
pub async fn subgroups_of(
    conn: &mut PgConnection,
    group_ids: &[Uuid],
) -> Result<Vec<Subgroup>, sqlx::Error> {
    sqlx::query_as!(
        Subgroup,
        "SELECT id, group_id, name FROM org.subgroups WHERE group_id = ANY($1) ORDER BY id",
        group_ids
    )
    .fetch_all(conn)
    .await
}

pub async fn rename_subgroup(
    conn: &mut PgConnection,
    id: Uuid,
    name: &str,
) -> Result<Subgroup, sqlx::Error> {
    sqlx::query_as!(
        Subgroup,
        "UPDATE org.subgroups SET name = $2 WHERE id = $1 RETURNING id, group_id, name",
        id,
        name
    )
    .fetch_one(conn)
    .await
}

pub async fn delete_subgroup(conn: &mut PgConnection, id: Uuid) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM org.subgroups WHERE id = $1", id)
        .execute(conn)
        .await?;
    Ok(result.rows_affected())
}

// ---------------------------------------------------------------------------------------------
// Subgroup members
// ---------------------------------------------------------------------------------------------

/// Adds the users who are not members of the subgroup yet and returns them. A user who is not a
/// member of the group violates the composite key; the service has checked that under the lock.
pub async fn insert_subgroup_members(
    conn: &mut PgConnection,
    subgroup_id: Uuid,
    group_id: Uuid,
    user_ids: &[Uuid],
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"INSERT INTO org.subgroup_members (subgroup_id, group_id, user_id)
           SELECT $1, $2, u FROM UNNEST($3::uuid[]) AS u
           ON CONFLICT DO NOTHING
           RETURNING user_id"#,
        subgroup_id,
        group_id,
        user_ids
    )
    .fetch_all(conn)
    .await
}

pub async fn delete_subgroup_member(
    conn: &mut PgConnection,
    subgroup_id: Uuid,
    user_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM org.subgroup_members WHERE subgroup_id = $1 AND user_id = $2",
        subgroup_id,
        user_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// Same order and cursor as [`list_group_members`].
pub async fn list_subgroup_members(
    conn: &mut PgConnection,
    subgroup_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<GroupMember>, sqlx::Error> {
    sqlx::query_as!(
        GroupMember,
        r#"SELECT u.id AS "user_id!", u.name AS "name!", u.surname AS "surname!"
           FROM org.subgroup_members m
           JOIN auth.people u ON u.id = m.user_id
           WHERE m.subgroup_id = $1
             AND ($2::uuid IS NULL
                  OR (u.surname, u.name, u.id) > (SELECT c.surname, c.name, c.id
                                                  FROM auth.people c WHERE c.id = $2))
           ORDER BY u.surname, u.name, u.id
           LIMIT $3"#,
        subgroup_id,
        after,
        limit
    )
    .fetch_all(conn)
    .await
}

// ---------------------------------------------------------------------------------------------
// Download links of the desktop app (ADR-0064)
// ---------------------------------------------------------------------------------------------

/// The links in the order the platforms are offered.
pub async fn list_app_downloads(
    conn: &mut PgConnection,
) -> Result<Vec<(String, String)>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT platform, url FROM org.app_downloads
         ORDER BY array_position(
             ARRAY['windows', 'macos_arm', 'macos_intel', 'linux_deb', 'linux_appimage'], platform)"
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.platform, row.url))
        .collect())
}

/// Makes the table hold exactly `links`: the platforms that are not in it are removed, the others set.
pub async fn replace_app_downloads(
    conn: &mut PgConnection,
    links: &[(String, String)],
) -> Result<(), sqlx::Error> {
    let platforms: Vec<String> = links.iter().map(|(platform, _)| platform.clone()).collect();
    let urls: Vec<String> = links.iter().map(|(_, url)| url.clone()).collect();
    sqlx::query!(
        "DELETE FROM org.app_downloads WHERE platform <> ALL($1::text[])",
        &platforms
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query!(
        "INSERT INTO org.app_downloads (platform, url)
         SELECT * FROM UNNEST($1::text[], $2::text[])
         ON CONFLICT (platform) DO UPDATE SET url = EXCLUDED.url, updated_at = now()
         WHERE org.app_downloads.url IS DISTINCT FROM EXCLUDED.url",
        &platforms,
        &urls
    )
    .execute(conn)
    .await?;
    Ok(())
}
