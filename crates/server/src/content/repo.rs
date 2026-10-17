//! SQL only. Every function takes the connection it runs on and never begins a transaction.

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sqlx::PgConnection;
use sqlx::types::Json;
use uuid::Uuid;

use sqlx::types::Decimal;

use super::model::{
    Assignment, AssignmentSummary, BuiltinType, Item, ItemKind, ItemStatus, ItemSummary, MaxPoints,
    Module, ModuleStatus,
};
use crate::policy::Release;

type Body = Json<Map<String, Value>>;

/// A module together with the release state of its parent, if it has one.
pub struct ModuleRow {
    pub module: Module,
    pub parent: Option<Release>,
}

/// An item together with the release state of its module and of that module's parent, if any.
pub struct ItemRow {
    pub item: Item,
    pub module: Release,
    pub module_parent: Option<Release>,
}

/// One row of the tree query: a module, its parent's state and at most one of its items.
pub struct TreeRow {
    pub module: Module,
    pub parent: Option<Release>,
    pub item: Option<ItemSummary>,
}

struct RawItem {
    id: Uuid,
    module_id: Uuid,
    course_id: Uuid,
    position: i32,
    kind: ItemKind,
    title: String,
    body: Body,
    status: ItemStatus,
    publish_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<RawItem> for Item {
    fn from(raw: RawItem) -> Self {
        Self {
            id: raw.id,
            module_id: raw.module_id,
            course_id: raw.course_id,
            position: raw.position,
            kind: raw.kind,
            title: raw.title,
            body: raw.body.0,
            status: raw.status,
            publish_at: raw.publish_at,
            assignment: None,
            created_at: raw.created_at,
            updated_at: raw.updated_at,
        }
    }
}

/// The columns of `content.assignments` as a LEFT JOIN returns them: all `None` for a `text` item.
struct JoinedAssignment {
    assignment_type: Option<BuiltinType>,
    config: Option<Body>,
    max_points: Option<MaxPoints>,
    is_bonus: Option<bool>,
}

impl JoinedAssignment {
    fn full(self) -> Option<Assignment> {
        Some(Assignment {
            assignment_type: self.assignment_type?,
            config: self.config?.0,
            max_points: self.max_points?,
            is_bonus: self.is_bonus?,
        })
    }

    fn summary(self) -> Option<AssignmentSummary> {
        Some(AssignmentSummary {
            assignment_type: self.assignment_type?,
            max_points: self.max_points?,
            is_bonus: self.is_bonus?,
        })
    }
}

fn release(status: Option<ModuleStatus>, publish_at: Option<DateTime<Utc>>) -> Option<Release> {
    status.map(|status| Release {
        published: status == ModuleStatus::Published,
        publish_at,
    })
}

// ---------------------------------------------------------------------------------------------
// Modules
// ---------------------------------------------------------------------------------------------

/// Inserts a module as the last of its siblings: the top-level modules of the course when
/// `parent_id` is `None`, otherwise the sub-modules of that parent. Two concurrent inserts can
/// pick the same position; the deferred unique constraint then fails one of them at commit.
pub async fn insert_module(
    conn: &mut PgConnection,
    id: Uuid,
    course_id: Uuid,
    parent_id: Option<Uuid>,
    title: &str,
) -> Result<Module, sqlx::Error> {
    sqlx::query_as!(
        Module,
        r#"INSERT INTO content.modules (id, course_id, parent_id, position, title)
           VALUES ($1, $2, $3::uuid,
                   COALESCE((SELECT max(position) FROM content.modules
                             WHERE course_id = $2 AND parent_id IS NOT DISTINCT FROM $3::uuid), 0) + 1,
                   $4)
           RETURNING id, course_id, parent_id, position, title,
                     status AS "status: ModuleStatus", publish_at, created_at, updated_at"#,
        id,
        course_id,
        parent_id,
        title
    )
    .fetch_one(conn)
    .await
}

pub async fn find_module(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<ModuleRow>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT m.id, m.course_id, m.parent_id, m.position, m.title,
                  m.status AS "status: ModuleStatus", m.publish_at, m.created_at, m.updated_at,
                  p.status AS "parent_status?: ModuleStatus", p.publish_at AS parent_publish_at
           FROM content.modules m
           LEFT JOIN content.modules p ON p.id = m.parent_id
           WHERE m.id = $1"#,
        id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|row| ModuleRow {
        module: Module {
            id: row.id,
            course_id: row.course_id,
            parent_id: row.parent_id,
            position: row.position,
            title: row.title,
            status: row.status,
            publish_at: row.publish_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        },
        parent: release(row.parent_status, row.parent_publish_at),
    }))
}

/// `title` is kept when `None`; `publish_at` is changed only when `set_publish_at` is true.
pub async fn update_module(
    conn: &mut PgConnection,
    id: Uuid,
    title: Option<&str>,
    set_publish_at: bool,
    publish_at: Option<DateTime<Utc>>,
) -> Result<Module, sqlx::Error> {
    sqlx::query_as!(
        Module,
        r#"UPDATE content.modules
           SET title = COALESCE($2::text, title),
               publish_at = CASE WHEN $3 THEN $4::timestamptz ELSE publish_at END,
               updated_at = now()
           WHERE id = $1
           RETURNING id, course_id, parent_id, position, title,
                     status AS "status: ModuleStatus", publish_at, created_at, updated_at"#,
        id,
        title,
        set_publish_at,
        publish_at
    )
    .fetch_one(conn)
    .await
}

pub async fn delete_module(conn: &mut PgConnection, id: Uuid) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM content.modules WHERE id = $1", id)
        .execute(conn)
        .await?;
    Ok(result.rows_affected())
}

/// The ids of one sibling sequence: the top-level modules of the course, or the sub-modules of a
/// parent.
pub async fn module_ids(
    conn: &mut PgConnection,
    course_id: Uuid,
    parent_id: Option<Uuid>,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT id FROM content.modules
         WHERE course_id = $1 AND parent_id IS NOT DISTINCT FROM $2::uuid",
        course_id,
        parent_id
    )
    .fetch_all(conn)
    .await
}

/// Gives `ids[0]` position 1, `ids[1]` position 2 and so on, within one sibling sequence.
/// Returns how many rows were rewritten.
pub async fn reorder_modules(
    conn: &mut PgConnection,
    course_id: Uuid,
    parent_id: Option<Uuid>,
    ids: &[Uuid],
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "UPDATE content.modules m SET position = o.pos::int, updated_at = now()
         FROM unnest($3::uuid[]) WITH ORDINALITY AS o(id, pos)
         WHERE m.id = o.id AND m.course_id = $1 AND m.parent_id IS NOT DISTINCT FROM $2::uuid",
        course_id,
        parent_id,
        ids
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// Publishes a draft module. `None` when it was not a draft, so nothing is touched.
pub async fn publish_module(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<Module>, sqlx::Error> {
    sqlx::query_as!(
        Module,
        r#"UPDATE content.modules SET status = 'published', updated_at = now()
           WHERE id = $1 AND status = 'draft'
           RETURNING id, course_id, parent_id, position, title,
                     status AS "status: ModuleStatus", publish_at, created_at, updated_at"#,
        id
    )
    .fetch_optional(conn)
    .await
}

pub async fn publish_draft_submodules(
    conn: &mut PgConnection,
    parent_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "UPDATE content.modules SET status = 'published', updated_at = now()
         WHERE parent_id = $1 AND status = 'draft'",
        parent_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// Publishes the draft items of a module and of its sub-modules.
pub async fn publish_draft_items_below(
    conn: &mut PgConnection,
    module_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "UPDATE content.items SET status = 'published', updated_at = now()
         WHERE status = 'draft'
           AND (module_id = $1 OR module_id IN (SELECT id FROM content.modules WHERE parent_id = $1))",
        module_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

// ---------------------------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------------------------

/// Inserts an item as the last of its module.
pub async fn insert_item(
    conn: &mut PgConnection,
    id: Uuid,
    module_id: Uuid,
    course_id: Uuid,
    kind: ItemKind,
    title: &str,
    body: &Map<String, Value>,
) -> Result<Item, sqlx::Error> {
    let raw = sqlx::query_as!(
        RawItem,
        r#"INSERT INTO content.items (id, module_id, course_id, position, kind, title, body)
           VALUES ($1, $2, $3,
                   COALESCE((SELECT max(position) FROM content.items WHERE module_id = $2), 0) + 1,
                   $4, $5, $6)
           RETURNING id, module_id, course_id, position, kind AS "kind: ItemKind", title,
                     body AS "body: Body", status AS "status: ItemStatus", publish_at,
                     created_at, updated_at"#,
        id,
        module_id,
        course_id,
        kind.as_str(),
        title,
        Json(body) as Json<&Map<String, Value>>
    )
    .fetch_one(conn)
    .await?;
    Ok(raw.into())
}

pub async fn find_item(conn: &mut PgConnection, id: Uuid) -> Result<Option<ItemRow>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT i.id, i.module_id, i.course_id, i.position, i.kind AS "kind: ItemKind", i.title,
                  i.body AS "body: Body", i.status AS "status: ItemStatus", i.publish_at,
                  i.created_at, i.updated_at,
                  m.status AS "module_status: ModuleStatus", m.publish_at AS module_publish_at,
                  p.status AS "parent_status?: ModuleStatus", p.publish_at AS parent_publish_at,
                  a.type AS "assignment_type?: BuiltinType", a.config AS "assignment_config?: Body",
                  a.max_points AS "assignment_max_points?: MaxPoints",
                  a.is_bonus AS "assignment_is_bonus?"
           FROM content.items i
           JOIN content.modules m ON m.id = i.module_id
           LEFT JOIN content.modules p ON p.id = m.parent_id
           LEFT JOIN content.assignments a ON a.item_id = i.id
           WHERE i.id = $1"#,
        id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|row| ItemRow {
        item: Item {
            id: row.id,
            module_id: row.module_id,
            course_id: row.course_id,
            position: row.position,
            kind: row.kind,
            title: row.title,
            body: row.body.0,
            status: row.status,
            publish_at: row.publish_at,
            assignment: JoinedAssignment {
                assignment_type: row.assignment_type,
                config: row.assignment_config,
                max_points: row.assignment_max_points,
                is_bonus: row.assignment_is_bonus,
            }
            .full(),
            created_at: row.created_at,
            updated_at: row.updated_at,
        },
        module: Release {
            published: row.module_status == ModuleStatus::Published,
            publish_at: row.module_publish_at,
        },
        module_parent: release(row.parent_status, row.parent_publish_at),
    }))
}

/// `title` and `body` are kept when `None`; `publish_at` is changed only when `set_publish_at`.
pub async fn update_item(
    conn: &mut PgConnection,
    id: Uuid,
    title: Option<&str>,
    body: Option<&Map<String, Value>>,
    set_publish_at: bool,
    publish_at: Option<DateTime<Utc>>,
) -> Result<Item, sqlx::Error> {
    let raw = sqlx::query_as!(
        RawItem,
        r#"UPDATE content.items
           SET title = COALESCE($2::text, title),
               body = COALESCE($3::jsonb, body),
               publish_at = CASE WHEN $4 THEN $5::timestamptz ELSE publish_at END,
               updated_at = now()
           WHERE id = $1
           RETURNING id, module_id, course_id, position, kind AS "kind: ItemKind", title,
                     body AS "body: Body", status AS "status: ItemStatus", publish_at,
                     created_at, updated_at"#,
        id,
        title,
        body.map(Json) as Option<Json<&Map<String, Value>>>,
        set_publish_at,
        publish_at
    )
    .fetch_one(conn)
    .await?;
    Ok(raw.into())
}

pub async fn delete_item(conn: &mut PgConnection, id: Uuid) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM content.items WHERE id = $1", id)
        .execute(conn)
        .await?;
    Ok(result.rows_affected())
}

pub async fn item_ids(conn: &mut PgConnection, module_id: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT id FROM content.items WHERE module_id = $1",
        module_id
    )
    .fetch_all(conn)
    .await
}

/// Gives `ids[0]` position 1, `ids[1]` position 2 and so on. Returns how many rows were rewritten.
pub async fn reorder_items(
    conn: &mut PgConnection,
    module_id: Uuid,
    ids: &[Uuid],
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "UPDATE content.items i SET position = o.pos::int, updated_at = now()
         FROM unnest($2::uuid[]) WITH ORDINALITY AS o(id, pos)
         WHERE i.id = o.id AND i.module_id = $1",
        module_id,
        ids
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// Publishes a draft item. `None` when it was not a draft, so nothing is touched.
pub async fn publish_item(conn: &mut PgConnection, id: Uuid) -> Result<Option<Item>, sqlx::Error> {
    let raw = sqlx::query_as!(
        RawItem,
        r#"UPDATE content.items SET status = 'published', updated_at = now()
           WHERE id = $1 AND status = 'draft'
           RETURNING id, module_id, course_id, position, kind AS "kind: ItemKind", title,
                     body AS "body: Body", status AS "status: ItemStatus", publish_at,
                     created_at, updated_at"#,
        id
    )
    .fetch_optional(conn)
    .await?;
    Ok(raw.map(Item::from))
}

// ---------------------------------------------------------------------------------------------
// The tree
// ---------------------------------------------------------------------------------------------

/// Every module of the course with its items, one row per (module, item) and one row for a module
/// without items. Top-level modules come in position order, each directly followed by its
/// sub-modules in position order; the items of one module come in position order.
///
/// With `open_at`, only rows a student may see at that moment are returned: the module and its
/// parent are published and not scheduled later, and so is each item. `policy` checks every row
/// again; this filter keeps a course page from loading what it would then drop.
pub async fn tree(
    conn: &mut PgConnection,
    course_id: Uuid,
    open_at: Option<DateTime<Utc>>,
) -> Result<Vec<TreeRow>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT m.id, m.course_id, m.parent_id, m.position, m.title,
                  m.status AS "status: ModuleStatus", m.publish_at, m.created_at, m.updated_at,
                  p.status AS "parent_status?: ModuleStatus", p.publish_at AS parent_publish_at,
                  i.id AS "item_id?", i.kind AS "item_kind?: ItemKind", i.title AS "item_title?",
                  i.status AS "item_status?: ItemStatus", i.publish_at AS item_publish_at,
                  i.position AS "item_position?",
                  a.type AS "assignment_type?: BuiltinType",
                  a.max_points AS "assignment_max_points?: MaxPoints",
                  a.is_bonus AS "assignment_is_bonus?"
           FROM content.modules m
           LEFT JOIN content.modules p ON p.id = m.parent_id
           LEFT JOIN content.items i
                  ON i.module_id = m.id
                 AND ($2::timestamptz IS NULL
                      OR (i.status = 'published' AND (i.publish_at IS NULL OR i.publish_at <= $2)))
           LEFT JOIN content.assignments a ON a.item_id = i.id
           WHERE m.course_id = $1
             AND ($2::timestamptz IS NULL
                  OR (m.status = 'published' AND (m.publish_at IS NULL OR m.publish_at <= $2)
                      AND (p.id IS NULL
                           OR (p.status = 'published' AND (p.publish_at IS NULL OR p.publish_at <= $2)))))
           ORDER BY COALESCE(p.position, m.position), m.parent_id NULLS FIRST, m.position, i.position"#,
        course_id,
        open_at
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| TreeRow {
            module: Module {
                id: row.id,
                course_id: row.course_id,
                parent_id: row.parent_id,
                position: row.position,
                title: row.title,
                status: row.status,
                publish_at: row.publish_at,
                created_at: row.created_at,
                updated_at: row.updated_at,
            },
            parent: release(row.parent_status, row.parent_publish_at),
            item: match (row.item_id, row.item_kind, row.item_title, row.item_status) {
                (Some(id), Some(kind), Some(title), Some(status)) => Some(ItemSummary {
                    id,
                    kind,
                    title,
                    status,
                    publish_at: row.item_publish_at,
                    position: row.item_position.unwrap_or_default(),
                    assignment: JoinedAssignment {
                        assignment_type: row.assignment_type,
                        config: None,
                        max_points: row.assignment_max_points,
                        is_bonus: row.assignment_is_bonus,
                    }
                    .summary(),
                }),
                _ => None,
            },
        })
        .collect())
}

// ---------------------------------------------------------------------------------------------
// Assignments
// ---------------------------------------------------------------------------------------------

struct RawAssignment {
    assignment_type: BuiltinType,
    config: Body,
    max_points: MaxPoints,
    is_bonus: bool,
}

impl From<RawAssignment> for Assignment {
    fn from(raw: RawAssignment) -> Self {
        Self {
            assignment_type: raw.assignment_type,
            config: raw.config.0,
            max_points: raw.max_points,
            is_bonus: raw.is_bonus,
        }
    }
}

/// Adds the assignment row of an item of kind `assignment`. The database refuses any other item.
pub async fn insert_assignment(
    conn: &mut PgConnection,
    item_id: Uuid,
    assignment: &Assignment,
) -> Result<Assignment, sqlx::Error> {
    let raw = sqlx::query_as!(
        RawAssignment,
        r#"INSERT INTO content.assignments (item_id, type, config, max_points, is_bonus)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING type AS "assignment_type: BuiltinType", config AS "config: Body",
                     max_points AS "max_points: MaxPoints", is_bonus"#,
        item_id,
        assignment.assignment_type.as_str(),
        Json(&assignment.config) as Json<&Map<String, Value>>,
        assignment.max_points.value(),
        assignment.is_bonus
    )
    .fetch_one(conn)
    .await?;
    Ok(raw.into())
}

pub async fn find_assignment(
    conn: &mut PgConnection,
    item_id: Uuid,
) -> Result<Option<Assignment>, sqlx::Error> {
    let raw = sqlx::query_as!(
        RawAssignment,
        r#"SELECT type AS "assignment_type: BuiltinType", config AS "config: Body",
                  max_points AS "max_points: MaxPoints", is_bonus
           FROM content.assignments WHERE item_id = $1"#,
        item_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(raw.map(Assignment::from))
}

/// The assignment row, locked `FOR UPDATE` until the transaction ends. A grade write holds the
/// same row `FOR SHARE`, so this waits for it (and it for us), and the checks that come in a
/// later statement see every grade the writer committed (ADR-0028). It returns the row as it is
/// after the wait. `None` when the item is not an assignment or is gone.
pub async fn lock_assignment(
    conn: &mut PgConnection,
    item_id: Uuid,
) -> Result<Option<Assignment>, sqlx::Error> {
    let raw = sqlx::query_as!(
        RawAssignment,
        r#"SELECT type AS "assignment_type: BuiltinType", config AS "config: Body",
                  max_points AS "max_points: MaxPoints", is_bonus
           FROM content.assignments WHERE item_id = $1
           FOR UPDATE"#,
        item_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(raw.map(Assignment::from))
}

/// Locks every assignment of a module and of its sub-modules `FOR UPDATE`, in id order so that
/// two deletes never wait for each other in a circle. Returns their ids.
pub async fn lock_assignments_below(
    conn: &mut PgConnection,
    module_id: Uuid,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT a.item_id FROM content.assignments a
         JOIN content.items i ON i.id = a.item_id
         JOIN content.modules m ON m.id = i.module_id
         WHERE m.id = $1 OR m.parent_id = $1
         ORDER BY a.item_id
         FOR UPDATE OF a",
        module_id
    )
    .fetch_all(conn)
    .await
}

/// Each field is kept when `None`.
pub async fn update_assignment(
    conn: &mut PgConnection,
    item_id: Uuid,
    config: Option<&Map<String, Value>>,
    max_points: Option<MaxPoints>,
    is_bonus: Option<bool>,
) -> Result<Assignment, sqlx::Error> {
    let raw = sqlx::query_as!(
        RawAssignment,
        r#"UPDATE content.assignments
           SET config = COALESCE($2::jsonb, config),
               max_points = COALESCE($3::numeric, max_points),
               is_bonus = COALESCE($4::boolean, is_bonus),
               updated_at = now()
           WHERE item_id = $1
           RETURNING type AS "assignment_type: BuiltinType", config AS "config: Body",
                     max_points AS "max_points: MaxPoints", is_bonus"#,
        item_id,
        config.map(Json) as Option<Json<&Map<String, Value>>>,
        max_points.map(MaxPoints::value),
        is_bonus
    )
    .fetch_one(conn)
    .await?;
    Ok(raw.into())
}

/// Takes the transaction-scoped advisory lock that serializes writes to a course's points
/// (ADR-0022), waiting for the holder to commit or roll back. It is released at the end of the
/// transaction. The course id is hashed into the lock key; two courses that collide only wait for
/// each other. Callers read the sum in a later statement, so READ COMMITTED sees the holder's rows.
pub async fn lock_course_points(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"SELECT pg_advisory_xact_lock(hashtextextended('content.points:' || $1::uuid::text, 0)) AS "locked!: ()""#,
        course_id
    )
    .fetch_one(conn)
    .await?;
    Ok(())
}

/// The sum of `max_points` over the course's regular (not bonus) assignments, drafts included.
/// `except` leaves one assignment out: the one being changed.
pub async fn regular_points(
    conn: &mut PgConnection,
    course_id: Uuid,
    except: Option<Uuid>,
) -> Result<Decimal, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT COALESCE(sum(a.max_points), 0) AS "sum!"
           FROM content.assignments a
           JOIN content.items i ON i.id = a.item_id
           WHERE i.course_id = $1 AND NOT a.is_bonus AND a.item_id IS DISTINCT FROM $2::uuid"#,
        course_id,
        except
    )
    .fetch_one(conn)
    .await
}

/// How many sessions of this database wait for an advisory lock. Tests poll it to know that a
/// writer is blocked behind another one, instead of sleeping and hoping.
///
/// A plain runtime query: it reads a system view, not our schema, so there is nothing for sqlx to
/// check.
#[cfg(test)]
pub async fn advisory_lock_waiters(conn: &mut PgConnection) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM pg_locks
         WHERE locktype = 'advisory' AND NOT granted
           AND database = (SELECT oid FROM pg_database WHERE datname = current_database())",
    )
    .fetch_one(conn)
    .await
}
