//! SQL only. Every function takes the connection it runs on and never begins a transaction.

use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{Map, Value};
use sqlx::PgConnection;
use sqlx::types::Json;
use uuid::Uuid;

use sqlx::types::Decimal;

use super::model::{
    Assignment, AssignmentSummary, AssignmentType, BuiltinType, Item, ItemKind, ItemStatus,
    ItemSummary, MaxPoints, Module, ModuleStatus, OnDeadline, PublishedSlot, StaffFacts,
    SubmissionSettings, WorkFacts, WorkStatus,
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
    published_title: Option<String>,
    published_body: Option<Body>,
    last_published_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// The three columns of the published slot are set together (a CHECK of the table).
fn published_slot(
    title: Option<String>,
    body: Option<Body>,
    at: Option<DateTime<Utc>>,
) -> Option<PublishedSlot> {
    Some(PublishedSlot {
        title: title?,
        body: body?.0,
        at: at?,
    })
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
            published: published_slot(
                raw.published_title,
                raw.published_body,
                raw.last_published_at,
            ),
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
    settings: SubmissionSettings,
    class_on: Option<NaiveDate>,
}

impl JoinedAssignment {
    fn full(self) -> Option<Assignment> {
        Some(Assignment {
            assignment_type: self.assignment_type?,
            config: self.config?.0,
            max_points: self.max_points?,
            is_bonus: self.is_bonus?,
            settings: self.settings,
            class_on: self.class_on,
        })
    }

    fn summary(self) -> Option<AssignmentSummary> {
        Some(AssignmentSummary {
            assignment_type: self.assignment_type?,
            max_points: self.max_points?,
            is_bonus: self.is_bonus?,
            settings: self.settings,
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
) -> Result<Option<Module>, sqlx::Error> {
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
    .fetch_optional(conn)
    .await
}

pub async fn delete_module(conn: &mut PgConnection, id: Uuid) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM content.modules WHERE id = $1", id)
        .execute(conn)
        .await?;
    Ok(result.rows_affected())
}

/// Renumbers one sibling sequence of modules 1, 2, 3 … in the order it has, so that a delete leaves
/// no gap (ADR-0020). The unique key on the position is deferred, so the rewrite may pass through
/// duplicates. The rows are not touched otherwise: their `updated_at` stays.
pub async fn compact_modules(
    conn: &mut PgConnection,
    course_id: Uuid,
    parent_id: Option<Uuid>,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "UPDATE content.modules m SET position = r.rn
         FROM (SELECT id, (row_number() OVER (ORDER BY position))::int AS rn
               FROM content.modules
               WHERE course_id = $1 AND parent_id IS NOT DISTINCT FROM $2::uuid) r
         WHERE m.id = r.id AND m.position <> r.rn",
        course_id,
        parent_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// Like [`compact_modules`] for the items of one module.
pub async fn compact_items(conn: &mut PgConnection, module_id: Uuid) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "UPDATE content.items i SET position = r.rn
         FROM (SELECT id, (row_number() OVER (ORDER BY position))::int AS rn
               FROM content.items WHERE module_id = $1) r
         WHERE i.id = r.id AND i.position <> r.rn",
        module_id
    )
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
        "UPDATE content.items
         SET status = 'published', published_title = title, published_body = body,
             last_published_at = now(), updated_at = now()
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
                     published_title, published_body AS "published_body?: Body",
                     last_published_at, created_at, updated_at"#,
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
                  i.published_title, i.published_body AS "published_body?: Body",
                  i.last_published_at,
                  i.created_at, i.updated_at,
                  m.status AS "module_status: ModuleStatus", m.publish_at AS module_publish_at,
                  p.status AS "parent_status?: ModuleStatus", p.publish_at AS parent_publish_at,
                  a.type AS "assignment_type?: BuiltinType", a.config AS "assignment_config?: Body",
                  a.max_points AS "assignment_max_points?: MaxPoints",
                  a.is_bonus AS "assignment_is_bonus?",
                  a.class_on AS assignment_class_on,
                  a.due_at AS assignment_due_at,
                  a.on_deadline AS "assignment_on_deadline?: OnDeadline",
                  a.accept_late AS assignment_accept_late,
                  a.late_until AS assignment_late_until,
                  a.max_attempts AS assignment_max_attempts
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
                settings: SubmissionSettings {
                    due_at: row.assignment_due_at,
                    on_deadline: row.assignment_on_deadline,
                    accept_late: row.assignment_accept_late,
                    late_until: row.assignment_late_until,
                    max_attempts: row.assignment_max_attempts,
                },
                class_on: row.assignment_class_on,
            }
            .full(),
            published: published_slot(
                row.published_title,
                row.published_body,
                row.last_published_at,
            ),
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
) -> Result<Option<Item>, sqlx::Error> {
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
                     published_title, published_body AS "published_body?: Body",
                     last_published_at, created_at, updated_at"#,
        id,
        title,
        body.map(Json) as Option<Json<&Map<String, Value>>>,
        set_publish_at,
        publish_at
    )
    .fetch_optional(conn)
    .await?;
    Ok(raw.map(Item::from))
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

/// Publishes a draft item: its working title and body become what students read (ADR-0049).
/// `None` when it was not a draft, so nothing is touched.
pub async fn publish_item(conn: &mut PgConnection, id: Uuid) -> Result<Option<Item>, sqlx::Error> {
    let raw = sqlx::query_as!(
        RawItem,
        r#"UPDATE content.items
           SET status = 'published', published_title = title, published_body = body,
               last_published_at = now(), updated_at = now()
           WHERE id = $1 AND status = 'draft'
           RETURNING id, module_id, course_id, position, kind AS "kind: ItemKind", title,
                     body AS "body: Body", status AS "status: ItemStatus", publish_at,
                     published_title, published_body AS "published_body?: Body",
                     last_published_at, created_at, updated_at"#,
        id
    )
    .fetch_optional(conn)
    .await?;
    Ok(raw.map(Item::from))
}

/// Publishes the working title and body of a published item (ADR-0049). `None` when the item is
/// a draft or gone.
pub async fn publish_revision(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<Item>, sqlx::Error> {
    let raw = sqlx::query_as!(
        RawItem,
        r#"UPDATE content.items
           SET published_title = title, published_body = body, last_published_at = now(),
               updated_at = now()
           WHERE id = $1 AND status = 'published'
           RETURNING id, module_id, course_id, position, kind AS "kind: ItemKind", title,
                     body AS "body: Body", status AS "status: ItemStatus", publish_at,
                     published_title, published_body AS "published_body?: Body",
                     last_published_at, created_at, updated_at"#,
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
                  i.id AS "item_id?", i.kind AS "item_kind?: ItemKind",
                  CASE WHEN $2::timestamptz IS NULL THEN i.title ELSE i.published_title END
                      AS "item_title?",
                  i.status AS "item_status?: ItemStatus", i.publish_at AS item_publish_at,
                  i.position AS "item_position?",
                  a.type AS "assignment_type?: BuiltinType",
                  a.max_points AS "assignment_max_points?: MaxPoints",
                  a.is_bonus AS "assignment_is_bonus?",
                  a.due_at AS assignment_due_at,
                  a.on_deadline AS "assignment_on_deadline?: OnDeadline",
                  a.accept_late AS assignment_accept_late,
                  a.late_until AS assignment_late_until,
                  a.max_attempts AS assignment_max_attempts
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
                        settings: SubmissionSettings {
                            due_at: row.assignment_due_at,
                            on_deadline: row.assignment_on_deadline,
                            accept_late: row.assignment_accept_late,
                            late_until: row.assignment_late_until,
                            max_attempts: row.assignment_max_attempts,
                        },
                        class_on: None,
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

/// An assignment with its course and the release state of every row on the path to it, which
/// `policy` needs to decide whether a student may see it. No body: callers on a hot path (a draft
/// autosave) must not load a document they do not use.
pub struct AssignmentContext {
    pub course_id: Uuid,
    pub assignment_type: BuiltinType,
    pub item: Release,
    /// The module the item is in.
    pub module: Release,
    /// That module's parent, when the module is a sub-module.
    pub module_parent: Option<Release>,
}

pub async fn find_assignment_context(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<Option<AssignmentContext>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT i.course_id, a.type AS "assignment_type: BuiltinType",
                  i.status = 'published' AS "item_published!", i.publish_at AS item_publish_at,
                  m.status = 'published' AS "module_published!", m.publish_at AS module_publish_at,
                  p.status = 'published' AS "parent_published?", p.publish_at AS parent_publish_at
           FROM content.assignments a
           JOIN content.items i ON i.id = a.item_id
           JOIN content.modules m ON m.id = i.module_id
           LEFT JOIN content.modules p ON p.id = m.parent_id
           WHERE a.item_id = $1"#,
        assignment_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|row| AssignmentContext {
        course_id: row.course_id,
        assignment_type: row.assignment_type,
        item: Release {
            published: row.item_published,
            publish_at: row.item_publish_at,
        },
        module: Release {
            published: row.module_published,
            publish_at: row.module_publish_at,
        },
        module_parent: row.parent_published.map(|published| Release {
            published,
            publish_at: row.parent_publish_at,
        }),
    }))
}

struct RawAssignment {
    assignment_type: BuiltinType,
    config: Body,
    max_points: MaxPoints,
    is_bonus: bool,
    due_at: Option<DateTime<Utc>>,
    on_deadline: Option<OnDeadline>,
    accept_late: Option<bool>,
    late_until: Option<DateTime<Utc>>,
    max_attempts: Option<i32>,
    class_on: Option<NaiveDate>,
}

impl From<RawAssignment> for Assignment {
    fn from(raw: RawAssignment) -> Self {
        Self {
            assignment_type: raw.assignment_type,
            config: raw.config.0,
            max_points: raw.max_points,
            is_bonus: raw.is_bonus,
            settings: SubmissionSettings {
                due_at: raw.due_at,
                on_deadline: raw.on_deadline,
                accept_late: raw.accept_late,
                late_until: raw.late_until,
                max_attempts: raw.max_attempts,
            },
            class_on: raw.class_on,
        }
    }
}

/// Adds the assignment row of an item of kind `assignment`. The database refuses any other item.
pub async fn insert_assignment(
    conn: &mut PgConnection,
    item_id: Uuid,
    assignment: &Assignment,
) -> Result<Assignment, sqlx::Error> {
    let settings = &assignment.settings;
    let raw = sqlx::query_as!(
        RawAssignment,
        r#"INSERT INTO content.assignments
               (item_id, type, config, max_points, is_bonus,
                due_at, on_deadline, accept_late, late_until, max_attempts, class_on)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
           RETURNING type AS "assignment_type: BuiltinType", config AS "config: Body",
                     max_points AS "max_points: MaxPoints", is_bonus,
                     due_at, on_deadline AS "on_deadline: OnDeadline", accept_late, late_until,
                     max_attempts, class_on"#,
        item_id,
        assignment.assignment_type.as_str(),
        Json(&assignment.config) as Json<&Map<String, Value>>,
        assignment.max_points.value(),
        assignment.is_bonus,
        settings.due_at,
        settings.on_deadline.map(OnDeadline::as_str),
        settings.accept_late,
        settings.late_until,
        settings.max_attempts,
        assignment.class_on
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
                  max_points AS "max_points: MaxPoints", is_bonus,
                  due_at, on_deadline AS "on_deadline: OnDeadline", accept_late, late_until,
                  max_attempts, class_on
           FROM content.assignments WHERE item_id = $1"#,
        item_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(raw.map(Assignment::from))
}

/// The assignment row, locked `FOR UPDATE` until the transaction ends. A grade write and a
/// submission write hold the same row `FOR SHARE`, so this waits for them (and they for us), and
/// the checks that come in a later statement see everything they committed (ADR-0028,
/// ADR-0035). It returns the row as it is after the wait. `None` when the item is not an
/// assignment or is gone.
pub async fn lock_assignment(
    conn: &mut PgConnection,
    item_id: Uuid,
) -> Result<Option<Assignment>, sqlx::Error> {
    let raw = sqlx::query_as!(
        RawAssignment,
        r#"SELECT type AS "assignment_type: BuiltinType", config AS "config: Body",
                  max_points AS "max_points: MaxPoints", is_bonus,
                  due_at, on_deadline AS "on_deadline: OnDeadline", accept_late, late_until,
                  max_attempts, class_on
           FROM content.assignments WHERE item_id = $1
           FOR UPDATE"#,
        item_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(raw.map(Assignment::from))
}

/// Like [`lock_assignment`], but `FOR SHARE`: several holders at once, and a writer of the row
/// waits for all of them. It selects from `content.assignments` alone: a statement that had to
/// wait re-reads only the locked row, so what it returns is what the last writer committed.
pub async fn lock_assignment_shared(
    conn: &mut PgConnection,
    item_id: Uuid,
) -> Result<Option<Assignment>, sqlx::Error> {
    let raw = sqlx::query_as!(
        RawAssignment,
        r#"SELECT type AS "assignment_type: BuiltinType", config AS "config: Body",
                  max_points AS "max_points: MaxPoints", is_bonus,
                  due_at, on_deadline AS "on_deadline: OnDeadline", accept_late, late_until,
                  max_attempts, class_on
           FROM content.assignments WHERE item_id = $1
           FOR SHARE"#,
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

/// Locks every assignment of a course `FOR UPDATE`, in id order (see [`lock_assignments_below`]).
/// Returns their ids.
pub async fn lock_assignments_of_course(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT a.item_id FROM content.assignments a
         JOIN content.items i ON i.id = a.item_id
         WHERE i.course_id = $1
         ORDER BY a.item_id
         FOR UPDATE OF a",
        course_id
    )
    .fetch_all(conn)
    .await
}

/// What the course header shows about the content: the sum of the regular assignments' points
/// (bonus ones are extra and left out), how many there are, and when anything last changed. With
/// `open_at`, only what a student may see at that moment counts (the rule of [`tree`]).
pub async fn course_summary(
    conn: &mut PgConnection,
    course_id: Uuid,
    open_at: Option<DateTime<Utc>>,
) -> Result<(Decimal, i64, Option<DateTime<Utc>>), sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT COALESCE(sum(a.max_points) FILTER (WHERE NOT a.is_bonus), 0) AS "total!",
                  count(a.item_id) FILTER (WHERE NOT a.is_bonus) AS "graded!",
                  (SELECT max(updated_at) FROM (
                       SELECT updated_at FROM content.modules WHERE course_id = $1
                       UNION ALL
                       SELECT updated_at FROM content.items WHERE course_id = $1) changes)
                      AS changed
           FROM content.items i
           JOIN content.assignments a ON a.item_id = i.id
           JOIN content.modules m ON m.id = i.module_id
           LEFT JOIN content.modules p ON p.id = m.parent_id
           WHERE i.course_id = $1
             AND ($2::timestamptz IS NULL
                  OR (i.status = 'published' AND (i.publish_at IS NULL OR i.publish_at <= $2)
                      AND m.status = 'published' AND (m.publish_at IS NULL OR m.publish_at <= $2)
                      AND (p.id IS NULL
                           OR (p.status = 'published'
                               AND (p.publish_at IS NULL OR p.publish_at <= $2)))))"#,
        course_id,
        open_at
    )
    .fetch_one(conn)
    .await?;
    Ok((row.total, row.graded, row.changed))
}

/// Copies the whole tree of `from` into `to` (ADR-0048): modules, sub-modules, items and
/// assignments, all as drafts. One statement gives every module and item a new id, so a failure
/// leaves nothing half copied. What is not copied: publication (`publish_at`), deadlines, the late
/// rule and anything a student or a grader produced.
pub async fn copy_course(conn: &mut PgConnection, from: Uuid, to: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "WITH module_map AS (
             SELECT id AS old_id, uuidv7() AS new_id FROM content.modules WHERE course_id = $1),
         item_map AS (
             SELECT id AS old_id, uuidv7() AS new_id FROM content.items WHERE course_id = $1),
         new_modules AS (
             INSERT INTO content.modules (id, course_id, parent_id, position, title, status)
             SELECT mm.new_id, $2, pm.new_id, m.position, m.title, 'draft'
             FROM content.modules m
             JOIN module_map mm ON mm.old_id = m.id
             LEFT JOIN module_map pm ON pm.old_id = m.parent_id
             RETURNING id),
         new_items AS (
             INSERT INTO content.items (id, module_id, course_id, position, kind, title, body, status)
             SELECT im.new_id, mm.new_id, $2, i.position, i.kind, i.title, i.body, 'draft'
             FROM content.items i
             JOIN item_map im ON im.old_id = i.id
             JOIN module_map mm ON mm.old_id = i.module_id
             RETURNING id)
         INSERT INTO content.assignments (item_id, type, config, max_points, is_bonus, max_attempts)
         SELECT im.new_id, a.type, a.config, a.max_points, a.is_bonus, a.max_attempts
         FROM content.assignments a
         JOIN item_map im ON im.old_id = a.item_id",
        from,
        to
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Each field is kept when `None`. `settings` replaces all five settings together: the caller has
/// resolved them against the locked row.
pub async fn update_assignment(
    conn: &mut PgConnection,
    item_id: Uuid,
    config: Option<&Map<String, Value>>,
    max_points: Option<MaxPoints>,
    is_bonus: Option<bool>,
    settings: Option<SubmissionSettings>,
    class_on: Option<Option<NaiveDate>>,
) -> Result<Option<Assignment>, sqlx::Error> {
    let new = settings.unwrap_or_default();
    let raw = sqlx::query_as!(
        RawAssignment,
        r#"UPDATE content.assignments
           SET config = COALESCE($2::jsonb, config),
               max_points = COALESCE($3::numeric, max_points),
               is_bonus = COALESCE($4::boolean, is_bonus),
               due_at = CASE WHEN $5 THEN $6::timestamptz ELSE due_at END,
               on_deadline = CASE WHEN $5 THEN $7::text ELSE on_deadline END,
               accept_late = CASE WHEN $5 THEN $8::boolean ELSE accept_late END,
               late_until = CASE WHEN $5 THEN $9::timestamptz ELSE late_until END,
               max_attempts = CASE WHEN $5 THEN $10::integer ELSE max_attempts END,
               class_on = CASE WHEN $11 THEN $12::date ELSE class_on END,
               updated_at = now()
           WHERE item_id = $1
           RETURNING type AS "assignment_type: BuiltinType", config AS "config: Body",
                     max_points AS "max_points: MaxPoints", is_bonus,
                     due_at, on_deadline AS "on_deadline: OnDeadline", accept_late, late_until,
                     max_attempts, class_on"#,
        item_id,
        config.map(Json) as Option<Json<&Map<String, Value>>>,
        max_points.map(MaxPoints::value),
        is_bonus,
        settings.is_some(),
        new.due_at,
        new.on_deadline.map(OnDeadline::as_str),
        new.accept_late,
        new.late_until,
        new.max_attempts,
        class_on.is_some(),
        class_on.flatten()
    )
    .fetch_optional(conn)
    .await?;
    Ok(raw.map(Assignment::from))
}

// ---------------------------------------------------------------------------------------------
// Per-item facts (ADR-0049). Read models: they join the tables of `assessment`, `gradebook` and
// `teaching` and write nothing.
// ---------------------------------------------------------------------------------------------

/// The body of every `text` item of the course: the working one for staff, the published one for
/// students. An item students cannot read yet has no published body and is left out.
pub async fn lecture_bodies(
    conn: &mut PgConnection,
    course_id: Uuid,
    working_copy: bool,
) -> Result<Vec<(Uuid, Map<String, Value>)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT id, CASE WHEN $2 THEN body ELSE published_body END AS "body?: Body"
           FROM content.items WHERE course_id = $1 AND kind = 'text'"#,
        course_id,
        working_copy
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| Some((row.id, row.body?.0)))
        .collect())
}

/// The class days of the course's in-class assignments.
pub async fn class_days(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<Vec<(Uuid, NaiveDate)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT a.item_id, a.class_on AS "class_on!" FROM content.assignments a
           JOIN content.items i ON i.id = a.item_id
           WHERE i.course_id = $1 AND a.class_on IS NOT NULL"#,
        course_id
    )
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(|r| (r.item_id, r.class_on)).collect())
}

/// Published items whose working copy differs from what students read.
pub async fn edited_items(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT id FROM content.items
         WHERE course_id = $1 AND status = 'published'
           AND (title IS DISTINCT FROM published_title OR body IS DISTINCT FROM published_body)",
        course_id
    )
    .fetch_all(conn)
    .await
}

/// A student's standing on every assignment of the given courses (see [`WorkStatus::of`]). Only
/// published grade columns are selected: nothing here can show a draft value (AGENTS section 6).
/// The caller keeps the assignments the student may see.
pub async fn student_work(
    conn: &mut PgConnection,
    student_id: Uuid,
    course_ids: &[Uuid],
    now: DateTime<Utc>,
) -> Result<Vec<WorkFacts>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT a.item_id AS assignment_id, i.course_id,
                  a.type AS "assignment_type: BuiltinType",
                  COALESCE(a.due_at IS NOT NULL
                           AND CASE WHEN a.accept_late THEN a.late_until ELSE a.due_at END < $3,
                           false) AS "closed!",
                  (SELECT s.updated_at FROM assessment.submissions s
                   WHERE s.assignment_id = a.item_id AND s.student_id = $1
                     AND s.status = 'draft') AS draft_saved_at,
                  (SELECT count(*) FROM assessment.submissions s
                   WHERE s.assignment_id = a.item_id AND s.student_id = $1
                     AND s.status = 'submitted') AS "attempts!",
                  g.published_score,
                  (g.published_score IS NOT NULL OR g.published_feedback IS NOT NULL)
                      AS "graded!",
                  g.published_at
           FROM content.assignments a
           JOIN content.items i ON i.id = a.item_id
           LEFT JOIN gradebook.grades g ON g.assignment_id = a.item_id AND g.student_id = $1
           WHERE i.course_id = ANY($2)"#,
        student_id,
        course_ids,
        now
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| WorkFacts {
            assignment_id: row.assignment_id,
            course_id: row.course_id,
            status: WorkStatus::of(
                row.graded,
                row.assignment_type.takes_submissions(),
                row.attempts,
                row.draft_saved_at.is_some(),
                row.closed,
            ),
            score: row.published_score,
            published_at: row.published_at,
            draft_saved_at: row.draft_saved_at,
            attempts_used: row.attempts,
        })
        .collect())
}

/// An item that matched a search (ADR-0052).
pub struct ItemHit {
    pub id: Uuid,
    pub course_id: Uuid,
    pub kind: ItemKind,
    pub title: String,
    pub body: Json<Map<String, Value>>,
    pub status: ItemStatus,
    pub due_at: Option<DateTime<Utc>>,
    pub title_match: bool,
}

/// Items of the courses whose title or body text contains `pattern` (a `LIKE` pattern with the
/// user's text escaped). Staff read the working copy and every status; a student the published
/// copy of what students can open at `now`. Title matches come first.
///
/// ponytail: a scan of the bodies of the caller's courses; add a trigram or full-text index on the
/// text of the bodies when a search becomes slow.
pub async fn search_items(
    conn: &mut PgConnection,
    course_ids: &[Uuid],
    staff: bool,
    pattern: &str,
    now: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<ItemHit>, sqlx::Error> {
    sqlx::query_as!(
        ItemHit,
        r#"SELECT i.id, i.course_id, i.kind AS "kind: ItemKind",
                  CASE WHEN $2 THEN i.title ELSE i.published_title END AS "title!",
                  CASE WHEN $2 THEN i.body ELSE i.published_body END AS "body!: Json<Map<String, Value>>",
                  i.status AS "status: ItemStatus", a.due_at,
                  (CASE WHEN $2 THEN i.title ELSE i.published_title END) ILIKE $3 AS "title_match!"
           FROM content.items i
           JOIN content.modules m ON m.id = i.module_id
           LEFT JOIN content.modules p ON p.id = m.parent_id
           LEFT JOIN content.assignments a ON a.item_id = i.id
           WHERE i.course_id = ANY($1)
             AND ($2
                  OR (i.status = 'published' AND (i.publish_at IS NULL OR i.publish_at <= $4)
                      AND m.status = 'published' AND (m.publish_at IS NULL OR m.publish_at <= $4)
                      AND (p.id IS NULL
                           OR (p.status = 'published'
                               AND (p.publish_at IS NULL OR p.publish_at <= $4)))))
             AND ((CASE WHEN $2 THEN i.title ELSE i.published_title END) ILIKE $3
                  OR jsonb_path_query_array(CASE WHEN $2 THEN i.body ELSE i.published_body END,
                                            '$.**.text')::text ILIKE $3)
           ORDER BY 8 DESC, i.updated_at DESC, i.id
           LIMIT $5"#,
        course_ids,
        staff,
        pattern,
        now,
        limit
    )
    .fetch_all(conn)
    .await
}

/// What a student's work list shows of an assignment (ADR-0052).
pub struct OpenAssignmentRow {
    pub assignment_id: Uuid,
    pub course_id: Uuid,
    pub title: String,
    pub assignment_type: BuiltinType,
    pub due_at: Option<DateTime<Utc>>,
    pub class_on: Option<NaiveDate>,
    pub max_points: MaxPoints,
}

/// The assignments of the courses that students can open at `now`: the item, its module and its
/// parent are published and released. The title is the published one (ADR-0049).
pub async fn open_assignments(
    conn: &mut PgConnection,
    course_ids: &[Uuid],
    now: DateTime<Utc>,
) -> Result<Vec<OpenAssignmentRow>, sqlx::Error> {
    sqlx::query_as!(
        OpenAssignmentRow,
        r#"SELECT a.item_id AS assignment_id, i.course_id, i.published_title AS "title!",
                  a.type AS "assignment_type: BuiltinType", a.due_at, a.class_on,
                  a.max_points AS "max_points: MaxPoints"
           FROM content.assignments a
           JOIN content.items i ON i.id = a.item_id
           JOIN content.modules m ON m.id = i.module_id
           LEFT JOIN content.modules p ON p.id = m.parent_id
           WHERE i.course_id = ANY($1)
             AND i.status = 'published' AND (i.publish_at IS NULL OR i.publish_at <= $2)
             AND m.status = 'published' AND (m.publish_at IS NULL OR m.publish_at <= $2)
             AND (p.id IS NULL
                  OR (p.status = 'published' AND (p.publish_at IS NULL OR p.publish_at <= $2)))
           ORDER BY i.course_id, a.due_at NULLS LAST, i.id"#,
        course_ids,
        now
    )
    .fetch_all(conn)
    .await
}

/// What staff still have to do on each assignment of a course, over the current students. Reads the
/// draft grade columns, so only a staff path may call it.
pub async fn staff_counts(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<Vec<StaffFacts>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT a.item_id AS assignment_id,
                  (SELECT count(*) FROM teaching.effective_members m
                   WHERE m.course_id = $1 AND m.role = 'student') AS "graded_total!",
                  (SELECT count(DISTINCT s.student_id) FROM assessment.submissions s
                   JOIN teaching.effective_members m
                     ON m.course_id = $1 AND m.user_id = s.student_id AND m.role = 'student'
                   WHERE s.assignment_id = a.item_id AND s.status = 'submitted') AS "submitted!",
                  (SELECT count(DISTINCT s.student_id) FROM assessment.submissions s
                   JOIN teaching.effective_members m
                     ON m.course_id = $1 AND m.user_id = s.student_id AND m.role = 'student'
                   LEFT JOIN gradebook.grades g
                     ON g.assignment_id = s.assignment_id AND g.student_id = s.student_id
                   WHERE s.assignment_id = a.item_id AND s.status = 'submitted'
                     AND g.draft_score IS NULL) AS "to_grade!",
                  (SELECT count(*) FROM gradebook.grades g
                   JOIN teaching.effective_members m
                     ON m.course_id = $1 AND m.user_id = g.student_id AND m.role = 'student'
                   WHERE g.assignment_id = a.item_id
                     AND (g.draft_score, g.draft_feedback)
                         IS DISTINCT FROM (g.published_score, g.published_feedback))
                      AS "draft_grades!"
           FROM content.assignments a
           JOIN content.items i ON i.id = a.item_id
           WHERE i.course_id = $1"#,
        course_id
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| StaffFacts {
            assignment_id: row.assignment_id,
            submitted: row.submitted,
            to_grade: row.to_grade,
            draft_grades: row.draft_grades,
            graded_total: row.graded_total,
        })
        .collect())
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
