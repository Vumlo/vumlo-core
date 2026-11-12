//! SQL of the course settings and the groups (ADR-0089). Like the rest of `repo`, it reads the
//! tables of `content` with plain JOINs and writes only `gradebook.*`.

use sqlx::PgConnection;
use sqlx::types::Decimal;
use uuid::Uuid;

use crate::gradebook::layout::{Layout, Settings};

/// The settings of a course; a course that never set any has the defaults.
pub async fn settings(conn: &mut PgConnection, course_id: Uuid) -> Result<Settings, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT target, pass_mark, layout AS "layout: Layout"
           FROM gradebook.course_settings WHERE course_id = $1"#,
        course_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row
        .map(|row| Settings {
            target: row.target,
            pass_mark: row.pass_mark,
            layout: row.layout,
        })
        .unwrap_or_default())
}

pub async fn save_settings(
    conn: &mut PgConnection,
    course_id: Uuid,
    settings: &Settings,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"INSERT INTO gradebook.course_settings (course_id, target, pass_mark, layout)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT (course_id) DO UPDATE
           SET target = $2, pass_mark = $3, layout = $4, updated_at = now()"#,
        course_id,
        settings.target,
        settings.pass_mark,
        settings.layout.as_str()
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// A row of `gradebook.groups`.
#[derive(Debug, Clone)]
pub struct GroupRow {
    pub id: Uuid,
    /// Set for a group that stands for a top-level module.
    pub module_id: Option<Uuid>,
    pub title: Option<String>,
    pub cap: Option<Decimal>,
    pub best_k: Option<i32>,
    pub counts: bool,
    pub bonus: bool,
}

/// Every group row of the course: the teacher's own in their order, then the module ones.
pub async fn groups(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<Vec<GroupRow>, sqlx::Error> {
    sqlx::query_as!(
        GroupRow,
        r#"SELECT id, module_id, title, cap, best_k, counts, bonus
           FROM gradebook.groups WHERE course_id = $1
           ORDER BY position NULLS LAST, id"#,
        course_id
    )
    .fetch_all(conn)
    .await
}

/// A custom group of this course.
pub async fn find_custom_group(
    conn: &mut PgConnection,
    course_id: Uuid,
    id: Uuid,
) -> Result<Option<GroupRow>, sqlx::Error> {
    sqlx::query_as!(
        GroupRow,
        r#"SELECT id, module_id, title, cap, best_k, counts, bonus
           FROM gradebook.groups
           WHERE id = $1 AND course_id = $2 AND module_id IS NULL"#,
        id,
        course_id
    )
    .fetch_optional(conn)
    .await
}

/// The rule row of a top-level module of this course, if one was set.
pub async fn find_module_group(
    conn: &mut PgConnection,
    course_id: Uuid,
    module_id: Uuid,
) -> Result<Option<GroupRow>, sqlx::Error> {
    sqlx::query_as!(
        GroupRow,
        r#"SELECT id, module_id, title, cap, best_k, counts, bonus
           FROM gradebook.groups WHERE module_id = $1 AND course_id = $2"#,
        module_id,
        course_id
    )
    .fetch_optional(conn)
    .await
}

/// Is this a top-level module of the course?
pub async fn is_top_module(
    conn: &mut PgConnection,
    course_id: Uuid,
    module_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM content.modules
                          WHERE id = $1 AND course_id = $2 AND parent_id IS NULL) AS "top!""#,
        module_id,
        course_id
    )
    .fetch_one(conn)
    .await
}

/// The next place for a group of the teacher's own.
pub async fn next_position(conn: &mut PgConnection, course_id: Uuid) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT COALESCE(max(position), 0) + 1 AS "next!"
           FROM gradebook.groups WHERE course_id = $1 AND module_id IS NULL"#,
        course_id
    )
    .fetch_one(conn)
    .await
}

pub struct Rules {
    pub cap: Option<Decimal>,
    pub best_k: Option<i32>,
    pub counts: bool,
    pub bonus: bool,
}

pub async fn insert_custom_group(
    conn: &mut PgConnection,
    id: Uuid,
    course_id: Uuid,
    title: &str,
    position: i32,
    rules: &Rules,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"INSERT INTO gradebook.groups (id, course_id, title, position, cap, best_k, counts, bonus)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        id,
        course_id,
        title,
        position,
        rules.cap,
        rules.best_k,
        rules.counts,
        rules.bonus
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Sets the rules of a top-level module: creates its row or replaces the rules.
pub async fn save_module_group(
    conn: &mut PgConnection,
    id: Uuid,
    course_id: Uuid,
    module_id: Uuid,
    rules: &Rules,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"INSERT INTO gradebook.groups (id, course_id, module_id, cap, best_k, counts, bonus)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           ON CONFLICT (module_id) WHERE module_id IS NOT NULL
           DO UPDATE SET cap = $4, best_k = $5, counts = $6, bonus = $7"#,
        id,
        course_id,
        module_id,
        rules.cap,
        rules.best_k,
        rules.counts,
        rules.bonus
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Replaces the rules of a custom group, and its title when one is given.
pub async fn update_custom_group(
    conn: &mut PgConnection,
    id: Uuid,
    title: Option<&str>,
    rules: &Rules,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"UPDATE gradebook.groups
           SET title = COALESCE($2, title), cap = $3, best_k = $4, counts = $5, bonus = $6
           WHERE id = $1 AND module_id IS NULL"#,
        id,
        title,
        rules.cap,
        rules.best_k,
        rules.counts,
        rules.bonus
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Deletes a group; the assignments in it go back to "Not sorted".
pub async fn delete_group(
    conn: &mut PgConnection,
    course_id: Uuid,
    id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM gradebook.groups WHERE id = $1 AND course_id = $2",
        id,
        course_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// How many of these ids are assignments of the course.
pub async fn count_course_assignments(
    conn: &mut PgConnection,
    course_id: Uuid,
    ids: &[Uuid],
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM content.assignments a
           JOIN content.items i ON i.id = a.item_id
           WHERE i.course_id = $1 AND a.item_id = ANY($2)"#,
        course_id,
        ids
    )
    .fetch_one(conn)
    .await
}

/// Puts the assignments in a custom group, or back in "Not sorted" when `group_id` is `None`.
pub async fn place(
    conn: &mut PgConnection,
    assignment_ids: &[Uuid],
    group_id: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "DELETE FROM gradebook.group_items WHERE assignment_id = ANY($1)",
        assignment_ids
    )
    .execute(&mut *conn)
    .await?;
    if let Some(group_id) = group_id {
        sqlx::query!(
            "INSERT INTO gradebook.group_items (assignment_id, group_id)
             SELECT id, $2 FROM unnest($1::uuid[]) AS id",
            assignment_ids,
            group_id
        )
        .execute(conn)
        .await?;
    }
    Ok(())
}

/// Where an assignment sits: its top-level module and the custom group it is mapped to.
#[derive(Debug, Clone)]
pub struct Placement {
    pub assignment_id: Uuid,
    pub top_module: Uuid,
    pub top_module_title: String,
    pub group_id: Option<Uuid>,
}

/// Copies the settings, the groups and the placements of `from` into the course `to`, whose
/// modules and assignments are the copies `modules` and `items` name (`(old id, new id)`). A group
/// of a module the map does not know is left out; the groups of the teacher's own get new keys.
pub async fn copy_layout(
    conn: &mut PgConnection,
    from: Uuid,
    to: Uuid,
    modules: &[(Uuid, Uuid)],
    items: &[(Uuid, Uuid)],
) -> Result<(), sqlx::Error> {
    let (module_old, module_new): (Vec<Uuid>, Vec<Uuid>) = modules.iter().copied().unzip();
    let (item_old, item_new): (Vec<Uuid>, Vec<Uuid>) = items.iter().copied().unzip();
    sqlx::query!(
        "WITH module_map AS (
             SELECT * FROM unnest($3::uuid[], $4::uuid[]) AS t (old_id, new_id)),
         item_map AS (
             SELECT * FROM unnest($5::uuid[], $6::uuid[]) AS t (old_id, new_id)),
         group_map AS (
             SELECT g.id AS old_id, uuidv7() AS new_id
             FROM gradebook.groups g
             LEFT JOIN module_map mm ON mm.old_id = g.module_id
             WHERE g.course_id = $1 AND (g.module_id IS NULL OR mm.new_id IS NOT NULL)),
         new_settings AS (
             INSERT INTO gradebook.course_settings (course_id, target, pass_mark, layout)
             SELECT $2, target, pass_mark, layout
             FROM gradebook.course_settings WHERE course_id = $1
             RETURNING course_id),
         new_groups AS (
             INSERT INTO gradebook.groups
                 (id, course_id, module_id, title, position, cap, best_k, counts, bonus)
             SELECT gm.new_id, $2, mm.new_id, g.title, g.position, g.cap, g.best_k, g.counts, g.bonus
             FROM gradebook.groups g
             JOIN group_map gm ON gm.old_id = g.id
             LEFT JOIN module_map mm ON mm.old_id = g.module_id
             RETURNING id)
         INSERT INTO gradebook.group_items (assignment_id, group_id)
         SELECT im.new_id, gm.new_id
         FROM gradebook.group_items gi
         JOIN item_map im ON im.old_id = gi.assignment_id
         JOIN group_map gm ON gm.old_id = gi.group_id",
        from,
        to,
        &module_old,
        &module_new,
        &item_old,
        &item_new
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The placement of every assignment of the course, in the order of `repo::columns`.
pub async fn placements(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<Vec<Placement>, sqlx::Error> {
    sqlx::query_as!(
        Placement,
        r#"SELECT i.id AS assignment_id, COALESCE(p.id, m.id) AS "top_module!",
                  COALESCE(p.title, m.title) AS "top_module_title!", gi.group_id AS "group_id?"
           FROM content.assignments a
           JOIN content.items i ON i.id = a.item_id
           JOIN content.modules m ON m.id = i.module_id
           LEFT JOIN content.modules p ON p.id = m.parent_id
           LEFT JOIN gradebook.group_items gi ON gi.assignment_id = a.item_id
           WHERE i.course_id = $1
           ORDER BY COALESCE(p.position, m.position), m.parent_id NULLS FIRST, m.position,
                    i.position"#,
        course_id
    )
    .fetch_all(conn)
    .await
}
