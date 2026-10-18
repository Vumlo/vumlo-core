//! SQL only. Every function takes the connection it runs on and never begins a transaction.

use chrono::NaiveDate;
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{Group, GroupMember, MemberLine, Subgroup, Term, TermUsage};

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
        "SELECT u.id AS user_id, u.name, u.surname, u.email, m.added_at
         FROM org.group_members m
         JOIN identity.users u ON u.id = m.user_id
         WHERE m.group_id = $1
         ORDER BY u.surname, u.name, u.id",
        group_id
    )
    .fetch_all(conn)
    .await
}

/// Every term with the number of regular courses that use it, and where `today` stands.
pub async fn term_usage(
    conn: &mut PgConnection,
    today: NaiveDate,
) -> Result<Vec<TermUsage>, sqlx::Error> {
    sqlx::query_as!(
        TermUsage,
        r#"SELECT t.id AS term_id,
                  (SELECT count(*) FROM teaching.courses c
                   WHERE c.term_id = t.id AND c.kind = 'regular') AS "courses!",
                  ($1 BETWEEN t.starts_on AND t.ends_on) AS "current!",
                  (t.ends_on < $1) AS "ended!"
           FROM org.terms t
           ORDER BY t.starts_on DESC, t.id"#,
        today
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
/// The place of the cursor in the order is read from `identity.users` (a read-only JOIN, as the
/// gradebook roster does), so it also works for a user who has just left.
pub async fn list_group_members(
    conn: &mut PgConnection,
    group_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<GroupMember>, sqlx::Error> {
    sqlx::query_as!(
        GroupMember,
        r#"SELECT u.id AS user_id, u.name, u.surname
           FROM org.group_members m
           JOIN identity.users u ON u.id = m.user_id
           WHERE m.group_id = $1
             AND ($2::uuid IS NULL
                  OR (u.surname, u.name, u.id) > (SELECT c.surname, c.name, c.id
                                                  FROM identity.users c WHERE c.id = $2))
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
        r#"SELECT u.id AS user_id, u.name, u.surname
           FROM org.subgroup_members m
           JOIN identity.users u ON u.id = m.user_id
           WHERE m.subgroup_id = $1
             AND ($2::uuid IS NULL
                  OR (u.surname, u.name, u.id) > (SELECT c.surname, c.name, c.id
                                                  FROM identity.users c WHERE c.id = $2))
           ORDER BY u.surname, u.name, u.id
           LIMIT $3"#,
        subgroup_id,
        after,
        limit
    )
    .fetch_all(conn)
    .await
}
