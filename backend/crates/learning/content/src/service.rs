//! Business rules. Every function that acts for a user authorizes first, takes the connection it
//! runs on and never begins or commits a transaction.
//!
//! A module or item the actor may not view does not exist as far as they can tell (404): an
//! unknown id, an id in a course they cannot see and a draft they may not open all give the same
//! error. Only after that does `EditContent` decide between going ahead and 403.

use std::collections::{HashMap, HashSet};

use api_types::ProfileName;
use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;
use sqlx::types::Decimal;
use uuid::Uuid;

use super::document::{
    DocumentErrors, EffectiveProfiles, FileRef, ValidDocument, distinct_ids, file_refs_under,
    plain_text, validate_document,
};
use super::model::{
    Access, Assignment, AssignmentType, BuiltinType, ContentModule, ContentSubModule, ContentTitle,
    DraftItem, FactScope, Hit, HitKind, Item, ItemFacts, ItemKind, ItemPatch, ItemStatus,
    ItemSummary, MaxPoints, ModelError, Module, ModulePatch, NextDeadline, OnDeadline,
    OpenAssignment, OpenedAssignment, Revision, SettingProblem, SettingsPatch, SourceFact,
    Standing, SubmissionSettings, TemplateCounts,
};
use super::repo::{self, CopiedIds, ItemRow, ModuleRow};
use super::rules::Node;
use super::standing;
use super::types::form::{self, Holders};
use ::files::FileVerdict;
use access::Actor;
use access::{self, AccessError, Action, CourseAccess, CourseStatus, Release};
use events::{self, AuditEntry, Event, EventsError};
use teaching::{self, TeachingError};
use vumlo_config::GradingScale;

/// The most points one assignment can be worth on the `twelve` scale (ADR-0022).
const TWELVE_LIMIT: u32 = 12;

#[derive(Debug, thiserror::Error)]
pub enum ContentError {
    #[error(transparent)]
    Forbidden(#[from] AccessError),
    #[error(transparent)]
    Invalid(#[from] ModelError),
    #[error("the parent must be a top-level module of this course")]
    InvalidParent,
    #[error("ids must list exactly the current siblings, once each")]
    InvalidOrder,
    /// `config`, `max_points` or `is_bonus` sent for a `text` item.
    #[error("only an assignment has this field")]
    NotAnAssignment { field: &'static str },
    /// The grading scale does not allow these points (ADR-0022).
    /// The `body` is not a valid document of the profile `full` (ADR-0025).
    #[error("the body is not a valid document")]
    Document(DocumentErrors),
    #[error("{message}")]
    PointsLimit {
        field: &'static str,
        message: String,
    },
    /// The body of a `file` item is not `{file_id?, description?}` (ADR-0024).
    #[error("{message}")]
    FileBody {
        field: String,
        message: &'static str,
    },
    /// The `description` of a `file` item is not a valid document of the profile `full`.
    #[error("the description is not a valid document")]
    FileDescription(DocumentErrors),
    /// The file does not exist, or belongs to someone else's work (ADR-0069).
    #[error("the file {id} cannot be used here")]
    FileUnusable { id: Uuid, field: String },
    /// An `image` block names a file that is not a PNG, JPEG, GIF or WebP (ADR-0069).
    #[error("an image block needs a PNG, JPEG, GIF or WebP file")]
    NotAnImage { field: String },
    /// A document that was stored cannot be read by the parser that validated it: a fault of the
    /// server, never of the request.
    #[error("a stored document cannot be read")]
    StoredDocument,
    /// Publishing a `file` item whose published body would name no file.
    #[error("a file item needs a file before it is published")]
    FileMissing,
    /// No such module, or the actor may not see it.
    #[error("module not found")]
    ModuleNotFound,
    /// No such item, or the actor may not see it.
    #[error("item not found")]
    ItemNotFound,
    /// A concurrent change got in the way (a position taken, a parent deleted). Retry.
    #[error("a concurrent change conflicts with this one")]
    Conflict,
    /// A revision can only be published for an item that is published (ADR-0049).
    #[error("the item is a draft: publish it first")]
    ItemNotPublished,
    /// A condition of an access rule is not acceptable (ADR-0092); `field` is its path in the
    /// request.
    #[error("the access rule is not valid")]
    Rule {
        field: String,
        message: &'static str,
    },
    /// A rule keeps the student from this item until they have done what it asks (ADR-0092).
    #[error("the item is locked until its conditions are met")]
    Locked,
    /// An assignment is done by being submitted or graded, so it takes no mark (ADR-0092).
    #[error("an assignment is done when it is submitted or graded, not when it is marked")]
    CompletesItself,
    /// The settings break a rule once the change is applied (ADR-0035); one entry per field.
    #[error("the settings are invalid")]
    Settings(Vec<SettingProblem>),
    /// `max_points` would drop below a score already given (ADR-0021, ADR-0028).
    #[error("max_points cannot be lowered below a score that was already given")]
    BelowGivenScores,
    /// The search text is not 2 to 100 characters (ADR-0052).
    #[error("the search text must be 2 to 100 characters")]
    BadQuery,
    /// The change touches the part of a quiz or form that is frozen once work was submitted
    /// (ADR-0023, ADR-0050, ADR-0051).
    #[error("the questions or fields cannot change after work was submitted")]
    StructureFrozen,
    #[error(transparent)]
    Teaching(TeachingError),
    #[error(transparent)]
    Events(#[from] EventsError),
    #[error(transparent)]
    Db(sqlx::Error),
}

impl From<sqlx::Error> for ContentError {
    /// Positions are unique per sibling sequence and checked at commit, and the foreign keys tie
    /// rows to parents. A violation of either means a concurrent writer, not a bad request:
    /// the service validates parents and ids before it writes.
    fn from(error: sqlx::Error) -> Self {
        let code = error
            .as_database_error()
            .and_then(|e| e.code())
            .map(|code| code.into_owned());
        match code.as_deref() {
            Some("23505" | "23503") => Self::Conflict,
            _ => Self::Db(error),
        }
    }
}

impl From<TeachingError> for ContentError {
    fn from(error: TeachingError) -> Self {
        Self::Teaching(error)
    }
}

/// The release state of the rows above a target: the top-level module on its path and, if there
/// is one, the sub-module.
struct Path {
    module: Release,
    submodule: Option<Release>,
}

impl Path {
    /// The path to a target whose own release is `own` and whose parent module, if it has one,
    /// is `parent`. A target with a parent sits in a sub-module's place on the path.
    fn new(own: Release, parent: Option<Release>) -> Self {
        match parent {
            Some(parent) => Self {
                module: parent,
                submodule: Some(own),
            },
            None => Self {
                module: own,
                submodule: None,
            },
        }
    }

    fn of_module(row: &ModuleRow) -> Self {
        Self::new(row.module.release(), row.parent)
    }

    /// The path to an item: the module it is in and that module's parent.
    fn of_item(row: &ItemRow) -> Self {
        Self::new(row.module, row.module_parent)
    }

    fn view(&self, access: CourseAccess, item: Option<Release>, at: DateTime<Utc>) -> Action {
        Action::ViewContent {
            access,
            module: self.module,
            submodule: self.submodule,
            item,
            at,
        }
    }
}

/// The actor's access to the course, with an unknown or invisible course reported as the
/// caller's `not_found`.
async fn access_or(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    not_found: ContentError,
) -> Result<CourseAccess, ContentError> {
    match teaching::course_access(conn, actor, course_id).await {
        Ok(access) => Ok(access),
        Err(TeachingError::CourseNotFound) => Err(not_found),
        Err(other) => Err(other.into()),
    }
}

/// Loads a module the actor may view.
async fn open_module(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<(ModuleRow, CourseAccess), ContentError> {
    let row = repo::find_module(conn, id)
        .await?
        .ok_or(ContentError::ModuleNotFound)?;
    let access = access_or(
        conn,
        actor,
        row.module.course_id,
        ContentError::ModuleNotFound,
    )
    .await?;
    let view = Path::of_module(&row).view(access, None, Utc::now());
    access::authorize(actor, view).map_err(|_| ContentError::ModuleNotFound)?;
    Ok((row, access))
}

/// Loads a module the actor may view and change, with their access to its course.
async fn manage_module(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<(ModuleRow, CourseAccess), ContentError> {
    let (row, access) = open_module(conn, actor, id).await?;
    access::authorize(actor, Action::EditContent(access))?;
    Ok((row, access))
}

/// Loads an item the actor may view.
async fn open_item(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<(ItemRow, CourseAccess), ContentError> {
    let row = repo::find_item(conn, id)
        .await?
        .ok_or(ContentError::ItemNotFound)?;
    let access = access_or(conn, actor, row.item.course_id, ContentError::ItemNotFound).await?;
    let view = Path::of_item(&row).view(access, Some(row.item.release()), Utc::now());
    access::authorize(actor, view).map_err(|_| ContentError::ItemNotFound)?;
    Ok((row, access))
}

/// Loads an item the actor may view and change, with their access to its course.
async fn manage_item(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<(ItemRow, CourseAccess), ContentError> {
    let (row, access) = open_item(conn, actor, id).await?;
    access::authorize(actor, Action::EditContent(access))?;
    Ok((row, access))
}

/// May the actor change the item (`EditContent`)? An item they may not see is `ItemNotFound`; an
/// archived course is `Forbidden(Archived)`. For the upload of a file that the item will name.
pub async fn authorize_edit_item(
    conn: &mut PgConnection,
    actor: &Actor,
    item_id: Uuid,
) -> Result<(), ContentError> {
    manage_item(conn, actor, item_id).await.map(|_| ())
}

/// May the actor read the files the published copy (`published`) or the working copy of the item
/// names (ADR-0069)? The published copy follows `ViewContent`, so a student gets nothing from a
/// draft, a scheduled item or an unpublished module; the working copy needs `ManageContent`.
/// An item the actor may not see is `false`, never an error, and so is one that a rule still keeps
/// from them (ADR-0092).
pub async fn may_read_item_files(
    conn: &mut PgConnection,
    actor: &Actor,
    item_id: Uuid,
    published: bool,
) -> Result<bool, ContentError> {
    let (row, access) = match open_item(conn, actor, item_id).await {
        Ok(opened) => opened,
        Err(ContentError::ItemNotFound) => return Ok(false),
        Err(other) => return Err(other),
    };
    if is_locked(conn, actor, access, row.item.course_id, item_id).await? {
        return Ok(false);
    }
    Ok(published || sees_drafts(actor, access))
}

/// The course access of an actor who wants to change the course's content.
async fn manage_course(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<CourseAccess, ContentError> {
    let access = teaching::course_access(conn, actor, course_id).await?;
    access::authorize(actor, Action::EditContent(access))?;
    Ok(access)
}

/// `parent_id` must name a top-level module of `course_id`.
async fn check_parent(
    conn: &mut PgConnection,
    course_id: Uuid,
    parent_id: Uuid,
) -> Result<(), ContentError> {
    match repo::find_module(conn, parent_id).await? {
        Some(row) if row.module.course_id == course_id && row.module.parent_id.is_none() => Ok(()),
        _ => Err(ContentError::InvalidParent),
    }
}

/// `ids` must be exactly the members of `current`, each once.
fn check_order(ids: &[Uuid], current: &[Uuid]) -> Result<(), ContentError> {
    let wanted: HashSet<_> = ids.iter().collect();
    let have: HashSet<_> = current.iter().collect();
    if wanted.len() == ids.len() && wanted == have {
        Ok(())
    } else {
        Err(ContentError::InvalidOrder)
    }
}

/// A reorder that rewrote fewer rows than it listed raced with a delete.
fn check_rewritten(rewritten: u64, ids: &[Uuid]) -> Result<(), ContentError> {
    if usize::try_from(rewritten).is_ok_and(|n| n == ids.len()) {
        Ok(())
    } else {
        Err(ContentError::Conflict)
    }
}

async fn audit(
    conn: &mut PgConnection,
    actor: &Actor,
    action: &'static str,
    resource_type: &'static str,
    resource_id: Uuid,
    course_id: Uuid,
) -> Result<(), ContentError> {
    events::audit_admin(
        conn,
        actor,
        AuditEntry {
            action,
            resource_type,
            resource_id,
            meta: json!({"course_id": course_id}),
        },
    )
    .await?;
    Ok(())
}

/// Refuses points the grading scale does not allow (ADR-0022). The caller decides whether the
/// change needs the check at all: creating and raising `max_points` do; lowering and deleting
/// never do. On the `hundred` scale there is no limit on one assignment or on their sum: the
/// course total is cut at the target by the gradebook (ADR-0089).
fn check_points(
    scale: GradingScale,
    points: Decimal,
    field: &'static str,
) -> Result<(), ContentError> {
    if scale == GradingScale::Twelve && points > Decimal::from(TWELVE_LIMIT) {
        return Err(ContentError::PointsLimit {
            field,
            message: format!(
                "an assignment may be worth at most {TWELVE_LIMIT} points on the twelve-point scale"
            ),
        });
    }
    Ok(())
}

/// The item with the assignment part its kind calls for, read in the current transaction.
async fn with_assignment(conn: &mut PgConnection, mut item: Item) -> Result<Item, ContentError> {
    if item.kind == ItemKind::Assignment {
        item.assignment = repo::find_assignment(conn, item.id).await?;
    }
    Ok(item)
}

// ---------------------------------------------------------------------------------------------
// Modules
// ---------------------------------------------------------------------------------------------

/// Creates a draft module as the last of its siblings: a top-level module, or a sub-module when
/// `parent_id` is set.
pub async fn create_module(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    title: ContentTitle,
    parent_id: Option<Uuid>,
) -> Result<Module, ContentError> {
    manage_course(conn, actor, course_id).await?;
    if let Some(parent_id) = parent_id {
        check_parent(conn, course_id, parent_id).await?;
    }
    let module =
        repo::insert_module(conn, Uuid::now_v7(), course_id, parent_id, title.as_str()).await?;
    audit(
        conn,
        actor,
        "content.module.create",
        "module",
        module.id,
        course_id,
    )
    .await?;
    Ok(module)
}

/// What a course header shows about the content, as the actor may see it (ADR-0048).
pub struct CourseSummary {
    /// The sum of `max_points` of the regular assignments (the bonus ones are extra).
    pub total_points: sqlx::types::Decimal,
    pub graded_items: i64,
    /// When a module or an item last changed; `None` for a course with no content.
    pub changed_at: Option<chrono::DateTime<Utc>>,
}

/// The numbers of the course header. Staff count every assignment, others only those open to
/// students now (the rule of [`get_tree`]).
pub async fn course_summary(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<CourseSummary, ContentError> {
    let access = teaching::course_access(conn, actor, course_id).await?;
    let staff = sees_drafts(actor, access);
    let (total_points, graded_items, changed_at) =
        repo::course_summary(conn, course_id, (!staff).then(Utc::now)).await?;
    Ok(CourseSummary {
        total_points,
        graded_items,
        changed_at,
    })
}

/// Locks every assignment row of the course `FOR UPDATE`, for the delete of the course (ADR-0035
/// point 12): a grade write or a submit in flight holds one of them, so the checks that follow, in
/// later statements, see what it committed. The caller has authorized the delete.
pub async fn lock_course_assignments(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<(), ContentError> {
    repo::lock_assignments_of_course(conn, course_id).await?;
    Ok(())
}

/// Copies the whole tree of `from` into the empty course `to`, all as drafts (ADR-0048). The caller
/// has authorized both courses. Returns which new id each module and item got.
pub async fn copy_course_content(
    conn: &mut PgConnection,
    from: Uuid,
    to: Uuid,
) -> Result<CopiedIds, ContentError> {
    Ok(repo::copy_course(conn, from, to).await?)
}

/// The course's content as the actor may see it. Staff see everything; others only what is open
/// to students now. Not paged: a course tree is bounded (ADR-0020).
pub async fn get_tree(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<Vec<ContentModule>, ContentError> {
    let access = teaching::course_access(conn, actor, course_id).await?;
    let mut tree = load_tree(conn, actor, course_id, access).await?;
    attach_access(conn, actor, access, course_id, &mut tree).await?;
    Ok(tree)
}

/// Whether the actor reads the working copy of a course: drafts, scheduled items and the rules.
/// Staff do; a student reads what is open to them, and the rules only as where they stand.
pub fn sees_drafts(actor: &Actor, access: CourseAccess) -> bool {
    access::authorize(actor, Action::ManageContent(access)).is_ok()
}

/// Puts on every node of `tree` what its reader is told about the rules (ADR-0092): staff the rule
/// that locks it, a student where they stand.
async fn attach_access(
    conn: &mut PgConnection,
    actor: &Actor,
    access: CourseAccess,
    course_id: Uuid,
    tree: &mut [ContentModule],
) -> Result<(), sqlx::Error> {
    if sees_drafts(actor, access) {
        let rules = repo::rules_of_course(conn, course_id).await?;
        standing::attach(tree, |node| standing::staff_access(&rules, node));
    } else {
        let standings = standing::student_standings(conn, course_id, actor.user_id(), tree).await?;
        standing::attach(tree, |node| standing::student_access(&standings, node));
    }
    Ok(())
}

/// What the actor is told about one module or item: the rule that locks it (staff), or where the
/// student stands on it.
async fn node_access(
    conn: &mut PgConnection,
    actor: &Actor,
    access: CourseAccess,
    course_id: Uuid,
    node: Node,
) -> Result<Access, sqlx::Error> {
    if sees_drafts(actor, access) {
        let rules = repo::rules_of_course(conn, course_id).await?;
        return Ok(standing::staff_access(&rules, node));
    }
    let standings = standings_of(conn, actor, access, course_id).await?;
    Ok(standing::student_access(&standings, node))
}

/// Where a student stands on every module and item they can see in the course.
async fn standings_of(
    conn: &mut PgConnection,
    actor: &Actor,
    access: CourseAccess,
    course_id: Uuid,
) -> Result<HashMap<Node, Standing>, sqlx::Error> {
    let tree = load_tree(conn, actor, course_id, access).await?;
    standing::student_standings(conn, course_id, actor.user_id(), &tree).await
}

/// Whether a rule keeps the actor from the item. Never for staff, and never in a course that has
/// no rules, which is decided by one cheap query.
async fn is_locked(
    conn: &mut PgConnection,
    actor: &Actor,
    access: CourseAccess,
    course_id: Uuid,
    item_id: Uuid,
) -> Result<bool, sqlx::Error> {
    if sees_drafts(actor, access) || !repo::course_has_rules(conn, course_id).await? {
        return Ok(false);
    }
    let node = node_access(conn, actor, access, course_id, Node::Item(item_id)).await?;
    Ok(node
        .standing
        .is_some_and(|standing| !standing.locked.is_empty()))
}

/// The tree of the course as the actor may see it, without what the rules add. Staff see all of
/// it, a student only what is open to students now.
async fn load_tree(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    access: CourseAccess,
) -> Result<Vec<ContentModule>, sqlx::Error> {
    let now = Utc::now();
    let staff = sees_drafts(actor, access);
    let rows = repo::tree(conn, course_id, (!staff).then_some(now)).await?;

    let mut top_level = Vec::new();
    let mut submodules_of: HashMap<Uuid, Vec<Module>> = HashMap::new();
    let mut items_of = HashMap::new();
    let mut seen = HashSet::new();
    for row in rows {
        let path = Path::new(row.module.release(), row.parent);
        if access::authorize(actor, path.view(access, None, now)).is_err() {
            continue;
        }
        if seen.insert(row.module.id) {
            match row.module.parent_id {
                None => top_level.push(row.module.clone()),
                Some(parent_id) => submodules_of
                    .entry(parent_id)
                    .or_default()
                    .push(row.module.clone()),
            }
        }
        if let Some(item) = row.item
            && access::authorize(actor, path.view(access, Some(item.release()), now)).is_ok()
        {
            items_of
                .entry(row.module.id)
                .or_insert_with(Vec::new)
                .push(item);
        }
    }

    Ok(top_level
        .into_iter()
        .map(|module| {
            let submodules = submodules_of
                .remove(&module.id)
                .unwrap_or_default()
                .into_iter()
                .map(|sub| ContentSubModule {
                    items: items_of.remove(&sub.id).unwrap_or_default(),
                    module: sub,
                })
                .collect();
            ContentModule {
                items: items_of.remove(&module.id).unwrap_or_default(),
                submodules,
                module,
            }
        })
        .collect())
}

/// Changes the title and/or `publish_at`. A change with no fields writes nothing.
pub async fn update_module(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
    patch: ModulePatch,
) -> Result<Module, ContentError> {
    let (row, access) = manage_module(conn, actor, id).await?;
    let course_id = row.module.course_id;
    if patch.is_empty() {
        let mut module = row.module;
        module.access = node_access(conn, actor, access, course_id, Node::Module(id)).await?;
        return Ok(module);
    }
    let mut module = repo::update_module(
        conn,
        id,
        patch.title.as_ref().map(ContentTitle::as_str),
        patch.publish_at.is_some(),
        patch.publish_at.flatten(),
    )
    .await?
    // Deleted while this request waited for the row: it is gone, not a failure (ADR-0039).
    .ok_or(ContentError::ModuleNotFound)?;
    if let Some(inputs) = &patch.access_rule {
        let tree = load_tree(conn, actor, course_id, access).await?;
        standing::set_rule(conn, course_id, Node::Module(id), inputs, &tree).await?;
    }
    audit(
        conn,
        actor,
        "content.module.update",
        "module",
        id,
        course_id,
    )
    .await?;
    module.access = node_access(conn, actor, access, course_id, Node::Module(id)).await?;
    Ok(module)
}

/// A module the actor may delete, with every assignment below it locked: the state between
/// [`begin_delete_module`] and [`finish_delete_module`].
#[derive(Debug)]
pub struct ModuleDeletion {
    id: Uuid,
    course_id: Uuid,
    parent_id: Option<Uuid>,
}

impl ModuleDeletion {
    pub fn id(&self) -> Uuid {
        self.id
    }
}

/// First half of deleting a module with its sub-modules and items: authorizes, then locks every
/// assignment below it. A grade write in flight holds one of them, so the checks that follow, in
/// later statements, see its grade (ADR-0028); a submit holds the same row `FOR SHARE`, so an
/// attempt in flight is committed by then (ADR-0035). The flow (`learning::items::delete_module`)
/// refuses when grades or attempts exist, purges the empty rows and drafts of the other modules,
/// and calls [`finish_delete_module`].
pub async fn begin_delete_module(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<ModuleDeletion, ContentError> {
    let (row, _) = manage_module(conn, actor, id).await?;
    repo::lock_assignments_below(conn, id).await?;
    Ok(ModuleDeletion {
        id,
        course_id: row.module.course_id,
        parent_id: row.module.parent_id,
    })
}

/// Second half: deletes the module and closes the gap in its sequence.
pub async fn finish_delete_module(
    conn: &mut PgConnection,
    actor: &Actor,
    deletion: ModuleDeletion,
) -> Result<(), ContentError> {
    let ModuleDeletion {
        id,
        course_id,
        parent_id,
    } = deletion;
    // A delete that lost the race to another one found nothing: no answer of success and no
    // audit row for something that did not happen (ADR-0039).
    if repo::delete_module(conn, id).await? == 0 {
        return Err(ContentError::ModuleNotFound);
    }
    repo::compact_modules(conn, course_id, parent_id).await?;
    audit(
        conn,
        actor,
        "content.module.delete",
        "module",
        id,
        course_id,
    )
    .await?;
    Ok(())
}

/// Reorders one sequence of modules: the top-level ones when `parent_id` is `None`, otherwise the
/// sub-modules of that parent. `ids` must be the complete current list.
pub async fn reorder_modules(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    parent_id: Option<Uuid>,
    ids: &[Uuid],
) -> Result<(), ContentError> {
    manage_course(conn, actor, course_id).await?;
    if let Some(parent_id) = parent_id {
        check_parent(conn, course_id, parent_id).await?;
    }
    let current = repo::module_ids(conn, course_id, parent_id).await?;
    check_order(ids, &current)?;
    let rewritten = repo::reorder_modules(conn, course_id, parent_id, ids).await?;
    check_rewritten(rewritten, ids)?;
    match parent_id {
        Some(parent_id) => {
            audit(
                conn,
                actor,
                "content.module.reorder",
                "module",
                parent_id,
                course_id,
            )
            .await?
        }
        None => {
            audit(
                conn,
                actor,
                "content.module.reorder",
                "course",
                course_id,
                course_id,
            )
            .await?
        }
    }
    Ok(())
}

/// Publishes a module. With `cascade` also its sub-modules and every draft item below it.
/// Publishing what is already published changes nothing; `publish_at` is never touched.
pub async fn publish_module(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
    cascade: bool,
) -> Result<Module, ContentError> {
    let (row, access) = manage_module(conn, actor, id).await?;
    let published = repo::publish_module(conn, id).await?;
    if published.is_none() && repo::find_module(conn, id).await?.is_none() {
        return Err(ContentError::ModuleNotFound);
    }
    let mut changed = published.is_some();
    if cascade {
        let submodules = repo::publish_draft_submodules(conn, id).await?;
        let items = repo::publish_draft_items_below(conn, id).await?;
        for item in &items {
            sync_published_files(conn, item).await?;
        }
        changed |= submodules + items.len() as u64 > 0;
    }
    // Also without `cascade`: the module may be what hid assignments that were published already.
    announce_visible(conn, Utc::now(), Some(row.module.course_id)).await?;
    if changed {
        audit(
            conn,
            actor,
            "content.module.publish",
            "module",
            id,
            row.module.course_id,
        )
        .await?;
    }
    let mut module = published.unwrap_or(row.module);
    module.access = node_access(conn, actor, access, module.course_id, Node::Module(id)).await?;
    Ok(module)
}

// ---------------------------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------------------------

/// What the item writes read from the configuration: the grading scale (ADR-0022) and the
/// resolved editor profiles (ADR-0025). Both are resolved once, when the state is built.
#[derive(Debug, Clone, Copy)]
pub struct ItemRules<'a> {
    pub scale: GradingScale,
    pub profiles: &'a EffectiveProfiles,
}

/// Validates a body as a document of profile `full`. Called only after the actor was authorized.
fn document(
    body: Option<&Value>,
    profiles: &EffectiveProfiles,
) -> Result<Option<ValidDocument>, ContentError> {
    body.map(|value| validate_document(value, ProfileName::Full, profiles))
        .transpose()
        .map_err(ContentError::Document)
}

/// The keys the body of a `file` item may have.
const FILE_BODY_KEYS: [&str; 2] = ["file_id", "description"];

/// Validates the body of a `file` item: an object with at most `file_id` (a uuid) and
/// `description` (a document of profile `full`). The file itself is checked by `learning::files::check`.
fn file_body(
    body: &Value,
    profiles: &EffectiveProfiles,
) -> Result<Map<String, Value>, ContentError> {
    let Value::Object(given) = body else {
        return Err(ContentError::FileBody {
            field: "body".to_owned(),
            message: "the body of a file item must be an object",
        });
    };
    if let Some(unknown) = given
        .keys()
        .find(|key| !FILE_BODY_KEYS.contains(&key.as_str()))
    {
        return Err(ContentError::FileBody {
            field: format!("body.{unknown}"),
            message: "only `file_id` and `description` are allowed",
        });
    }
    let mut stored = Map::new();
    if let Some(file_id) = given.get("file_id") {
        let id = file_id.as_str().and_then(|text| text.parse::<Uuid>().ok());
        let Some(id) = id else {
            return Err(ContentError::FileBody {
                field: "body.file_id".to_owned(),
                message: "must be the id of an uploaded file",
            });
        };
        stored.insert("file_id".to_owned(), Value::String(id.to_string()));
    }
    if let Some(description) = given.get("description") {
        let valid = validate_document(description, ProfileName::Full, profiles)
            .map_err(ContentError::FileDescription)?;
        stored.insert(
            "description".to_owned(),
            Value::Object(valid.as_body().clone()),
        );
    }
    Ok(stored)
}

/// The body to store for an item of `kind`, validated as that kind needs. `None` when there is no
/// body to write.
fn validated_body(
    kind: ItemKind,
    body: Option<&Value>,
    profiles: &EffectiveProfiles,
) -> Result<Option<Map<String, Value>>, ContentError> {
    match kind {
        ItemKind::File => body.map(|value| file_body(value, profiles)).transpose(),
        ItemKind::Text | ItemKind::Assignment => {
            Ok(document(body, profiles)?.map(|valid| valid.as_body().clone()))
        }
    }
}

/// The file a `file` item's body names.
fn file_id_of(body: &Map<String, Value>) -> Option<Uuid> {
    body.get("file_id")?.as_str()?.parse().ok()
}

/// Every file an item of `kind` names: in its `body` (fields start with `body.`) and, for an
/// assignment, in the documents of its `config` (fields start with `config_field`). The `file`
/// item's own file and the blocks of its description both count (ADR-0069). The documents are read
/// with the validator's parser; one it cannot read is an error, never "no files".
fn item_file_refs(
    kind: ItemKind,
    body: &Map<String, Value>,
    config: Option<(BuiltinType, &Map<String, Value>)>,
    config_field: &str,
) -> Result<Vec<FileRef>, ContentError> {
    let stored = |errors: DocumentErrors| {
        let found = errors.iter().next().map(|e| (e.path.clone(), e.message));
        tracing::error!(?found, "content: a stored document cannot be read");
        ContentError::StoredDocument
    };
    let mut refs = Vec::new();
    match kind {
        ItemKind::File => {
            if let Some(id) = file_id_of(body) {
                refs.push(FileRef {
                    id,
                    path: "body.file_id".to_owned(),
                    image: false,
                });
            }
            if let Some(description) = body.get("description") {
                refs.extend(file_refs_under("body.description", description).map_err(stored)?);
            }
        }
        ItemKind::Text | ItemKind::Assignment => {
            refs.extend(file_refs_under("body", &Value::Object(body.clone())).map_err(stored)?)
        }
    }
    if let Some((assignment_type, config)) = config {
        refs.extend(
            assignment_type
                .config_file_refs(config)?
                .into_iter()
                .map(|file| FileRef {
                    path: format!("{config_field}.{}", file.path),
                    ..file
                }),
        );
    }
    Ok(refs)
}

/// The ids of the files an item names: what `content.item_files` holds for it (ADR-0069). Every
/// rewrite of those rows goes through this function.
fn referenced_file_ids(
    kind: ItemKind,
    body: &Map<String, Value>,
    config: Option<(BuiltinType, &Map<String, Value>)>,
) -> Result<Vec<Uuid>, ContentError> {
    Ok(distinct_ids(&item_file_refs(kind, body, config, "config")?))
}

/// What the check of the files (`learning::files::check`, ADR-0069) said about `refs`, as the
/// error of this write. A refusal names the field of the first file that cannot be used, or of the
/// first image block whose file is not a PNG, JPEG, GIF or WebP.
fn judge_files(refs: &[FileRef], verdict: &FileVerdict) -> Result<(), ContentError> {
    let field = |id: Uuid| {
        refs.iter()
            .find(|file| file.id == id)
            .map_or_else(|| "body".to_owned(), |file| file.path.clone())
    };
    if let Some(id) = verdict.unusable {
        return Err(ContentError::FileUnusable {
            id,
            field: field(id),
        });
    }
    if let Some(id) = verdict.not_image {
        return Err(ContentError::NotAnImage { field: field(id) });
    }
    // The check names every file it judged. One it did not judge (an empty verdict handed to a
    // write that names files) is refused, so a write finished without the check attaches nothing.
    if let Some(file) = refs
        .iter()
        .find(|file| !verdict.names.contains_key(&file.id))
    {
        return Err(ContentError::FileUnusable {
            id: file.id,
            field: file.path.clone(),
        });
    }
    Ok(())
}

/// The assignment type and config of `item`, loaded when the item does not carry them.
async fn type_and_config(
    conn: &mut PgConnection,
    item: &Item,
) -> Result<Option<(BuiltinType, Map<String, Value>)>, ContentError> {
    if item.kind != ItemKind::Assignment {
        return Ok(None);
    }
    let assignment = match &item.assignment {
        Some(assignment) => Some(assignment.clone()),
        None => repo::find_assignment(conn, item.id).await?,
    };
    Ok(assignment.map(|a| (a.assignment_type, a.config)))
}

/// Makes `content.item_files` name exactly what the item's working body and config name.
async fn sync_working_files(conn: &mut PgConnection, item: &Item) -> Result<(), ContentError> {
    let config = type_and_config(conn, item).await?;
    let ids = referenced_file_ids(
        item.kind,
        &item.body,
        config.as_ref().map(|(kind, config)| (*kind, config)),
    )?;
    repo::replace_item_files(conn, item.id, false, &ids).await?;
    Ok(())
}

/// Makes the published rows of `content.item_files` name exactly what the published body names,
/// for an item that was just published. A `file` item whose published body names no file is
/// refused: the error rolls back the caller's transaction, so nothing becomes visible. Checking
/// the body that was written, not the one read before, leaves no gap for a concurrent edit.
///
/// The config of an assignment has no published copy: students read it as it is, so its images
/// are published with the item (and [`update_item`] rewrites these rows when the config changes).
async fn sync_published_files(conn: &mut PgConnection, item: &Item) -> Result<(), ContentError> {
    let Some(slot) = &item.published else {
        return Ok(());
    };
    if item.kind == ItemKind::File && file_id_of(&slot.body).is_none() {
        return Err(ContentError::FileMissing);
    }
    let config = type_and_config(conn, item).await?;
    let ids = referenced_file_ids(
        item.kind,
        &slot.body,
        config.as_ref().map(|(kind, config)| (*kind, config)),
    )?;
    repo::replace_item_files(conn, item.id, true, &ids).await?;
    Ok(())
}

/// What a new draft item is made of (ADR-0021, ADR-0024): a `text` item, an `assignment` (with its
/// `assignment` part) or a `file` item (whose body is `{file_id?, description?}`; the file is
/// uploaded after the item exists, so a new one usually has none yet).
#[derive(Debug, Clone)]
pub struct NewItemInput {
    pub title: ContentTitle,
    pub kind: ItemKind,
    pub body: Option<Value>,
    pub assignment: Option<Assignment>,
}

impl NewItemInput {
    /// A `text` item, or an `assignment` when `assignment` is given.
    pub fn document(
        title: ContentTitle,
        body: Option<Value>,
        assignment: Option<Assignment>,
    ) -> Self {
        let kind = if assignment.is_some() {
            ItemKind::Assignment
        } else {
            ItemKind::Text
        };
        Self {
            title,
            kind,
            body,
            assignment,
        }
    }
}

/// A new item that passed everything `content` can judge by itself, with its id chosen: the state
/// between [`begin_create_item`] and [`finish_create_item`].
#[derive(Debug)]
pub struct ItemCreation {
    id: Uuid,
    module_id: Uuid,
    course_id: Uuid,
    kind: ItemKind,
    title: ContentTitle,
    body: Map<String, Value>,
    assignment: Option<Assignment>,
    files: Vec<FileRef>,
}

impl ItemCreation {
    /// The id the item will have: the owner of the files it names.
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// The files the new item names, in its body and its config.
    pub fn files(&self) -> &[FileRef] {
        &self.files
    }
}

/// First half of creating a draft item as the last of a module, which may be a top-level module or
/// a sub-module: authorizes, validates, and lists the files the item names. The flow
/// (`learning::items::create_item`) has them checked and calls [`finish_create_item`].
pub async fn begin_create_item(
    conn: &mut PgConnection,
    actor: &Actor,
    rules: ItemRules<'_>,
    module_id: Uuid,
    input: NewItemInput,
) -> Result<ItemCreation, ContentError> {
    let NewItemInput {
        title,
        kind,
        body,
        mut assignment,
    } = input;
    let (row, _) = manage_module(conn, actor, module_id).await?;
    let course_id = row.module.course_id;
    if let Some(assignment) = &mut assignment {
        let kind = assignment.assignment_type;
        kind.strip_computed(&mut assignment.config);
        kind.validate_config(&assignment.config, rules.profiles)?;
        check_points_total(kind, &assignment.config, assignment.max_points)?;
    }
    let body = validated_body(kind, body.as_ref(), rules.profiles)?;
    if let Some(assignment) = &assignment {
        check_points(rules.scale, assignment.max_points.value(), "max_points")?;
    }
    let body = body.unwrap_or_else(|| match kind {
        ItemKind::File => Map::new(),
        ItemKind::Text | ItemKind::Assignment => ValidDocument::empty().as_body().clone(),
    });
    let files = item_file_refs(
        kind,
        &body,
        assignment.as_ref().map(|a| (a.assignment_type, &a.config)),
        "assignment.config",
    )?;
    Ok(ItemCreation {
        id: Uuid::now_v7(),
        module_id,
        course_id,
        kind,
        title,
        body,
        assignment,
        files,
    })
}

/// Second half: raises what the check of the files said, then inserts the item and the rows that
/// name its files. The files were locked by the check, in the transaction that is still open.
pub async fn finish_create_item(
    conn: &mut PgConnection,
    actor: &Actor,
    creation: ItemCreation,
    files: &FileVerdict,
) -> Result<Item, ContentError> {
    let ItemCreation {
        id,
        module_id,
        course_id,
        kind,
        title,
        body,
        assignment,
        files: refs,
    } = creation;
    judge_files(&refs, files)?;
    let mut item =
        repo::insert_item(conn, id, module_id, course_id, kind, title.as_str(), &body).await?;
    if let Some(assignment) = &assignment {
        item.assignment = Some(repo::insert_assignment(conn, item.id, assignment).await?);
    }
    sync_working_files(conn, &item).await?;
    audit(
        conn,
        actor,
        "content.item.create",
        "item",
        item.id,
        item.course_id,
    )
    .await?;
    Ok(item)
}

/// Tells the plugins (`assignment.created`, ADR-0075) of every published assignment that students
/// can see at `now` and that was not announced yet, in the transaction of whatever made it
/// visible, and returns how many. A publish route calls it for its own course; the scheduler calls
/// it for all courses, which catches a `publish_at` that has passed (ADR-0091). Each assignment is
/// announced once, whichever of them gets there first.
pub async fn announce_visible(
    conn: &mut PgConnection,
    now: DateTime<Utc>,
    course_id: Option<Uuid>,
) -> Result<usize, ContentError> {
    let claimed = repo::claim_unannounced(conn, now, course_id).await?;
    for &(assignment_id, course_id) in &claimed {
        events::emit(
            conn,
            Event::AssignmentCreated {
                course_id,
                assignment_id,
            },
        )
        .await?;
    }
    Ok(claimed.len())
}

/// The profiles the document editor enforces, for building its toolbar. Any role may ask.
pub fn editor_profiles<'a>(
    actor: &Actor,
    profiles: &'a EffectiveProfiles,
) -> Result<&'a EffectiveProfiles, ContentError> {
    access::authorize(actor, Action::ViewEditorProfiles)?;
    Ok(profiles)
}

/// An item as the actor reads it, and what is left to add when it is a form.
pub struct ItemRead {
    pub item: Item,
    /// Set for a form that is open to the reader: its config still wants the facts that only
    /// `assessment` knows (see [`decorate_form`]).
    pub form: Option<FormView>,
}

/// Who reads a form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormView {
    pub staff: bool,
    pub viewer: Uuid,
}

/// Adds to the config of a form the facts of ADR-0051 point 5 that `assessment` supplies: which
/// fields have answers and who holds each exclusive option (staff see who, a student only that it
/// is taken).
pub fn decorate_form(
    item: &mut Item,
    view: FormView,
    answered: &HashSet<String>,
    holders: &Holders,
) {
    if let Some(assignment) = &mut item.assignment {
        form::decorate(
            &mut assignment.config,
            answered,
            holders,
            view.staff,
            view.viewer,
        );
    }
}

/// An item as the actor may read it: staff get the working copy and the rule that locks it, a
/// student the published title and body (ADR-0049) and where they stand on it. An item that a rule
/// still keeps closed to the student is its title only (ADR-0092). A form comes back with the
/// reader in `form`: the flow (`learning::items::get`) asks `assessment` for its facts and
/// [`decorate_form`] adds them.
pub async fn get_item(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<ItemRead, ContentError> {
    let (row, access) = open_item(conn, actor, id).await?;
    let staff = sees_drafts(actor, access);
    let course_id = row.item.course_id;
    let mut item = if staff {
        row.item
    } else {
        row.item.as_seen_by_students()
    };
    item.access = node_access(conn, actor, access, course_id, Node::Item(id)).await?;
    // A locked item is its title and nothing else until the rule is met: no text, no file, no
    // questions (ADR-0092).
    if item
        .access
        .standing
        .as_ref()
        .is_some_and(|standing| !standing.locked.is_empty())
    {
        item.body = Map::new();
        if let Some(assignment) = &mut item.assignment {
            assignment.config = Map::new();
        }
        return Ok(ItemRead { item, form: None });
    }
    let mut form = None;
    if let Some(assignment) = &mut item.assignment {
        let kind = assignment.assignment_type;
        // A student never receives the answer key (ADR-0023, ADR-0050 point 6).
        if !staff {
            assignment.config = kind.student_config(&assignment.config);
        }
        if kind == BuiltinType::Form {
            form = Some(FormView {
                staff,
                viewer: actor.user_id(),
            });
        }
    }
    Ok(ItemRead { item, form })
}

/// Whether a published item has changes students do not see, and since when they read what they
/// read. Staff only.
pub async fn get_revision(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<Revision, ContentError> {
    let (row, access) = open_item(conn, actor, id).await?;
    access::authorize(actor, Action::ManageContent(access))?;
    Ok(Revision::of(&row.item))
}

/// Publishes the working title and body of a published item (ADR-0049). A draft has nothing to
/// publish a revision of: publish the item.
pub async fn publish_revision(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<Revision, ContentError> {
    let (row, _) = manage_item(conn, actor, id).await?;
    if row.item.status != ItemStatus::Published {
        return Err(ContentError::ItemNotPublished);
    }
    let item = repo::publish_revision(conn, id)
        .await?
        .ok_or(ContentError::ItemNotFound)?;
    sync_published_files(conn, &item).await?;
    if item.kind == ItemKind::Assignment {
        events::emit(
            conn,
            Event::AssignmentUpdated {
                course_id: item.course_id,
                assignment_id: item.id,
            },
        )
        .await?;
    }
    audit(
        conn,
        actor,
        "content.item.revision_publish",
        "item",
        id,
        item.course_id,
    )
    .await?;
    Ok(Revision::of(&item))
}

/// Words per minute of a reading time (ADR-0049).
const WORDS_PER_MINUTE: usize = 200;

/// The minutes a lecture takes to read: its words over 200, rounded up, at least one.
fn read_minutes(body: &serde_json::Map<String, serde_json::Value>) -> Option<i64> {
    let document: api_types::Document =
        serde_json::from_value(serde_json::Value::Object(body.clone())).ok()?;
    let words = plain_text(&document).split_whitespace().count();
    i64::try_from(words.div_ceil(WORDS_PER_MINUTE).max(1)).ok()
}

/// The facts `content` owns about every item of the course that the actor may see (ADR-0049,
/// point 5): the reading time, the class day, and for staff which items have unpublished changes.
/// `gradebook::item_meta` adds the standing and the counts. Not paged: a course tree is bounded
/// (ADR-0020).
pub async fn item_meta(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<HashMap<Uuid, ItemFacts>, ContentError> {
    let access = teaching::course_access(conn, actor, course_id).await?;
    let staff = sees_drafts(actor, access);
    let tree = get_tree(conn, actor, course_id).await?;
    let items: Vec<&ItemSummary> = tree
        .iter()
        .flat_map(|top| {
            top.items
                .iter()
                .chain(top.submodules.iter().flat_map(|s| s.items.iter()))
        })
        .collect();
    let mut facts: HashMap<Uuid, ItemFacts> = items
        .iter()
        .map(|item| (item.id, ItemFacts::default()))
        .collect();
    // How long a text takes to read is a fact about the text: not told of an item still locked.
    let locked: HashSet<Uuid> = items
        .iter()
        .filter(|item| {
            item.access
                .standing
                .as_ref()
                .is_some_and(|standing| !standing.locked.is_empty())
        })
        .map(|item| item.id)
        .collect();
    for (id, body) in repo::lecture_bodies(conn, course_id, staff).await? {
        if let Some(entry) = facts.get_mut(&id).filter(|_| !locked.contains(&id)) {
            entry.read_minutes = read_minutes(&body);
        }
    }
    for (id, day) in repo::class_days(conn, course_id).await? {
        if let Some(entry) = facts.get_mut(&id) {
            entry.class_on = Some(day);
        }
    }
    if staff {
        for id in repo::edited_items(conn, course_id).await? {
            if let Some(entry) = facts.get_mut(&id) {
                entry.edited = Some(true);
            }
        }
    }
    Ok(facts)
}

/// The assignments of `course_ids` that students can open at `now` (the item, its module and its
/// parent are published and released), with their published titles (ADR-0049). The caller has
/// authorized: it passes the courses the actor studies in.
pub async fn open_assignments(
    conn: &mut PgConnection,
    course_ids: &[Uuid],
    now: DateTime<Utc>,
) -> Result<Vec<OpenAssignment>, ContentError> {
    Ok(repo::open_assignments(conn, course_ids, now).await?)
}

/// What each of `course_ids` holds. The caller has authorized: it passes the templates the actor
/// may see.
pub async fn template_counts(
    conn: &mut PgConnection,
    course_ids: &[Uuid],
) -> Result<Vec<TemplateCounts>, ContentError> {
    Ok(repo::template_counts(conn, course_ids).await?)
}

/// The draft items inside published modules of `course_ids`, newest change first. The caller has
/// authorized: it passes the courses the actor works in as staff.
pub async fn draft_items(
    conn: &mut PgConnection,
    course_ids: &[Uuid],
) -> Result<Vec<DraftItem>, ContentError> {
    Ok(repo::draft_items(conn, course_ids).await?)
}

/// The earliest deadline from `now` on of any assignment, per course of `course_ids`. The caller
/// has authorized: it passes the courses the actor works in as staff.
pub async fn next_deadlines(
    conn: &mut PgConnection,
    course_ids: &[Uuid],
    now: DateTime<Utc>,
) -> Result<Vec<NextDeadline>, ContentError> {
    Ok(repo::next_deadlines(conn, course_ids, now).await?)
}

const SEARCH_MIN_CHARS: usize = 2;
const SEARCH_MAX_CHARS: usize = 100;
const SEARCH_LIMIT: usize = 50;
const SNIPPET_BEFORE: usize = 40;
const SNIPPET_AFTER: usize = 80;

/// `text` as a `LIKE` pattern that matches it as it is: `%`, `_` and `\` are escaped.
fn like_pattern(text: &str) -> String {
    let mut pattern = String::with_capacity(text.len() + 2);
    pattern.push('%');
    for c in text.chars() {
        if matches!(c, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    pattern
}

/// Every `text` string of a document, joined by spaces: what a reader reads, without the markup.
fn flat_text(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                match inner {
                    Value::String(text) if key == "text" => out.push(text.clone()),
                    other => flat_text(other, out),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| flat_text(item, out)),
        _ => {}
    }
}

/// A piece of `text` around the first match of `needle` (case-insensitive), with an ellipsis where
/// it was cut; `None` when `needle` is not in it. Works on characters, never on bytes.
fn snippet(text: &str, needle: &str) -> Option<String> {
    let flat: Vec<char> = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .collect();
    let lower: Vec<char> = flat.iter().flat_map(|c| c.to_lowercase()).collect();
    let needle: Vec<char> = needle.to_lowercase().chars().collect();
    // Lowercasing can change the length of a character; positions are only trusted when it did not.
    if lower.len() != flat.len() || needle.is_empty() {
        return None;
    }
    let at = lower
        .windows(needle.len())
        .position(|w| w == needle.as_slice())?;
    let from = at.saturating_sub(SNIPPET_BEFORE);
    let to = (at + needle.len() + SNIPPET_AFTER).min(flat.len());
    let body: String = flat[from..to].iter().collect();
    Some(format!(
        "{}{}{}",
        if from > 0 { "…" } else { "" },
        body,
        if to < flat.len() { "…" } else { "" }
    ))
}

/// Courses, lectures and assignments of the actor's courses that match `q` (ADR-0052). Students
/// read the published copy of what students can open; staff read the working copy and drafts.
/// Title matches come first; at most 50 hits.
pub async fn search(
    conn: &mut PgConnection,
    actor: &Actor,
    q: &str,
    now: DateTime<Utc>,
) -> Result<Vec<Hit>, ContentError> {
    let q = q.trim();
    if !(SEARCH_MIN_CHARS..=SEARCH_MAX_CHARS).contains(&q.chars().count()) {
        return Err(ContentError::BadQuery);
    }
    let scope = teaching::course_scope(conn, actor).await?;
    let lower = q.to_lowercase();
    let mut hits: Vec<(bool, Hit)> = Vec::new();
    for scoped in &scope {
        let course = &scoped.course;
        let in_title = course.course.title.to_lowercase().contains(&lower)
            || course.code.to_lowercase().contains(&lower);
        let described = snippet(&course.description, q);
        if in_title || described.is_some() {
            hits.push((
                in_title,
                Hit {
                    kind: HitKind::Course,
                    id: course.course.id,
                    title: course.course.title.clone(),
                    course_id: course.course.id,
                    course_title: course.course.title.clone(),
                    course_code: course.code.clone(),
                    snippet: if in_title { None } else { described },
                    due_at: None,
                    draft: course.course.status == CourseStatus::Draft,
                },
            ));
        }
    }
    let pattern = like_pattern(q);
    for staff in [true, false] {
        let ids: Vec<Uuid> = scope
            .iter()
            .filter(|scoped| scoped.staff == staff)
            .map(|scoped| scoped.course.course.id)
            .collect();
        if ids.is_empty() {
            continue;
        }
        let found =
            repo::search_items(conn, &ids, staff, &pattern, now, SEARCH_LIMIT as i64).await?;
        let locked = if staff {
            HashSet::new()
        } else {
            locked_items(conn, actor, &ids).await?
        };
        for item in found {
            let Some(scoped) = scope.iter().find(|s| s.course.course.id == item.course_id) else {
                continue;
            };
            // A locked item is its title until its rule is met: its text is neither matched nor
            // quoted (ADR-0092).
            if locked.contains(&item.id) && !item.title_match {
                continue;
            }
            let mut words = Vec::new();
            if !locked.contains(&item.id) {
                flat_text(&Value::Object(item.body.0), &mut words);
            }
            hits.push((
                item.title_match,
                Hit {
                    kind: match item.kind {
                        ItemKind::Text | ItemKind::File => HitKind::Lecture,
                        ItemKind::Assignment => HitKind::Assignment,
                    },
                    id: item.id,
                    title: item.title,
                    course_id: item.course_id,
                    course_title: scoped.course.course.title.clone(),
                    course_code: scoped.course.code.clone(),
                    snippet: if item.title_match {
                        None
                    } else {
                        snippet(&words.join(" "), q)
                    },
                    due_at: item.due_at,
                    draft: item.status == ItemStatus::Draft,
                },
            ));
        }
    }
    // A stable sort keeps the order the queries gave within each group.
    hits.sort_by_key(|(title_match, _)| !*title_match);
    Ok(hits
        .into_iter()
        .take(SEARCH_LIMIT)
        .map(|(_, hit)| hit)
        .collect())
}

/// The items, among those of the given courses, that a rule keeps from the actor (ADR-0092).
async fn locked_items(
    conn: &mut PgConnection,
    actor: &Actor,
    course_ids: &[Uuid],
) -> Result<HashSet<Uuid>, ContentError> {
    let mut locked = HashSet::new();
    for &course_id in course_ids {
        if !repo::course_has_rules(conn, course_id).await? {
            continue;
        }
        let access = teaching::course_access(conn, actor, course_id).await?;
        let standings = standings_of(conn, actor, access, course_id).await?;
        locked.extend(
            standings
                .into_iter()
                .filter_map(|(node, standing)| match node {
                    Node::Item(id) if !standing.locked.is_empty() => Some(id),
                    _ => None,
                }),
        );
    }
    Ok(locked)
}

/// Why [`open_assignment`] gave nothing.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// No such assignment, or the actor may not see it.
    #[error("assignment not found")]
    NotFound,
    #[error(transparent)]
    Teaching(TeachingError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// The one way another module opens an assignment for a user (ADR-0035, ADR-0030): the row, its
/// course, the actor's access to it, and the check that the actor may view it at all. An unknown
/// id, a course the actor cannot see and an item they may not open (a draft, a scheduled one) all
/// give `NotFound`. Nothing is locked; `lock_assignment_shared` is the next statement.
pub async fn open_assignment(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
) -> Result<OpenedAssignment, OpenError> {
    let row = repo::find_assignment_context(conn, assignment_id)
        .await?
        .ok_or(OpenError::NotFound)?;
    let access = match teaching::course_access(conn, actor, row.course_id).await {
        Ok(access) => access,
        Err(TeachingError::CourseNotFound) => return Err(OpenError::NotFound),
        Err(other) => return Err(OpenError::Teaching(other)),
    };
    let view = Path::new(row.module, row.module_parent).view(access, Some(row.item), Utc::now());
    access::authorize(actor, view).map_err(|_| OpenError::NotFound)?;
    let locked = is_locked(conn, actor, access, row.course_id, assignment_id).await?;
    Ok(OpenedAssignment {
        course_id: row.course_id,
        assignment_type: row.assignment_type,
        access,
        locked,
    })
}

/// Takes the assignment row `FOR SHARE` until the transaction ends and returns it as it is in the
/// row. A change of the row (points, settings) and a delete take it `FOR UPDATE`, so they wait for
/// this transaction, and what they did before is what this returns (ADR-0030, ADR-0035). A lock,
/// not a write. `None`: the assignment is gone.
pub async fn lock_assignment_shared(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<Option<Assignment>, sqlx::Error> {
    repo::lock_assignment_shared(conn, assignment_id).await
}

/// A type whose questions carry the points needs `max_points` to be their sum (ADR-0050 point 1).
fn check_points_total(
    kind: BuiltinType,
    config: &serde_json::Map<String, Value>,
    max_points: MaxPoints,
) -> Result<(), ContentError> {
    match kind.points_total(config) {
        Some(total) if total != max_points.value() => Err(ContentError::PointsLimit {
            field: "max_points",
            message: format!("must equal the sum of the points of the questions ({total:.2})"),
        }),
        _ => Ok(()),
    }
}

/// What an update of an assignment still has to know from the modules above `content`: whether
/// work was submitted, and the highest score given. It is read only when
/// [`ItemUpdate::asks_submissions`] and [`ItemUpdate::asks_scores`] say so; the other field stays at
/// its default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AssignmentFacts {
    pub has_submissions: bool,
    pub highest_score: Option<Decimal>,
}

/// An item update that passed everything `content` can judge by itself, with the assignment row
/// locked when the update touches points or config: the state between [`begin_update_item`] and
/// [`finish_update_item`].
pub struct ItemUpdate {
    id: Uuid,
    course_id: Uuid,
    access: CourseAccess,
    scale: GradingScale,
    /// An update of nothing: the item as it is.
    unchanged: Option<Item>,
    patch: ItemPatch,
    is_assignment: bool,
    body: Option<Map<String, Value>>,
    settings: Option<SubmissionSettings>,
    /// The deadline before this update, when the update can change it.
    was_due: Option<Option<DateTime<Utc>>>,
    /// The structure of the config changes, which is frozen once work was submitted (ADR-0023).
    changes_structure: bool,
    /// The points go down to this: no lower than a score already given (ADR-0021, ADR-0028).
    lowered_to: Option<MaxPoints>,
    /// The points go up to this: the grading scale limits them (ADR-0022).
    raised_to: Option<MaxPoints>,
    /// The files the new body and the new config name.
    files: Vec<FileRef>,
}

impl ItemUpdate {
    /// The files the update names, in the new body and the new config: the flow has them checked.
    pub fn files(&self) -> &[FileRef] {
        &self.files
    }

    /// The update needs `has_submissions` of [`AssignmentFacts`].
    pub fn asks_submissions(&self) -> bool {
        self.changes_structure
    }

    /// The update needs `highest_score` of [`AssignmentFacts`].
    pub fn asks_scores(&self) -> bool {
        self.lowered_to.is_some()
    }
}

/// First half of changing the title, body and/or `publish_at` of an item, and for an assignment its
/// `config`, `max_points` and `is_bonus`: authorizes, validates, and takes the assignment row lock
/// before the facts of other modules are asked, which is a later statement: a grade write holds the
/// row `FOR SHARE`, so this waits for it and the flow then sees its score. A submission write holds
/// it the same way, so a submit that began first finishes under the old settings and one that waits
/// sees the new ones (ADR-0035). The flow (`learning::items::update_item`) reads what the returned
/// update asks for and calls [`finish_update_item`]. A change with no fields writes nothing.
///
/// The row loaded to authorize may be stale by the time the points lock is granted (READ COMMITTED
/// lets another writer commit while this one waits). So whether `max_points` is raised, and
/// whether a bonus becomes regular, is decided on the assignment re-read after the lock is held.
pub async fn begin_update_item(
    conn: &mut PgConnection,
    actor: &Actor,
    rules: ItemRules<'_>,
    id: Uuid,
    mut patch: ItemPatch,
) -> Result<ItemUpdate, ContentError> {
    let (row, access) = manage_item(conn, actor, id).await?;
    let course_id = row.item.course_id;
    if patch.is_empty() {
        return Ok(ItemUpdate {
            id,
            course_id,
            access,
            scale: rules.scale,
            unchanged: Some(row.item),
            patch,
            is_assignment: false,
            body: None,
            settings: None,
            was_due: None,
            changes_structure: false,
            lowered_to: None,
            raised_to: None,
            files: Vec::new(),
        });
    }
    let is_assignment = row.item.kind == ItemKind::Assignment;
    if !is_assignment && patch.touches_assignment() {
        let field = if patch.config.is_some() {
            "config"
        } else if patch.max_points.is_some() {
            "max_points"
        } else if patch.is_bonus.is_some() {
            "is_bonus"
        } else if patch.class_on.is_some() {
            "class_on"
        } else {
            patch.settings.first_field()
        };
        return Err(ContentError::NotAnAssignment { field });
    }
    if let (Some(config), Some(assignment)) = (&mut patch.config, &row.item.assignment) {
        assignment.assignment_type.strip_computed(config);
    }
    let body = validated_body(row.item.kind, patch.body.as_ref(), rules.profiles)?;
    let mut files = Vec::new();
    if let Some(body) = &body {
        files = item_file_refs(row.item.kind, body, None, "config")?;
    }
    let mut settings = None;
    // The deadline before this update, when the update can change it.
    let mut was_due = None;
    let mut changes_structure = false;
    let mut lowered_to = None;
    let mut raised_to = None;
    if is_assignment && patch.touches_assignment() {
        let points_change = patch.max_points.is_some() || patch.is_bonus.is_some();
        // The current values are the ones in the locked row.
        let current = repo::lock_assignment(conn, id)
            .await?
            .ok_or(ContentError::ItemNotFound)?;
        was_due = Some(current.settings.due_at);
        if !patch.settings.is_empty() {
            settings = Some(
                current
                    .settings
                    .resolve(&patch.settings, current.assignment_type.takes_submissions())
                    .map_err(ContentError::Settings)?,
            );
        }
        if matches!(patch.class_on, Some(Some(_))) && current.assignment_type.takes_submissions() {
            return Err(ContentError::Settings(vec![SettingProblem {
                field: "class_on",
                message: "only an in-class assignment has a class day".to_owned(),
            }]));
        }
        if patch.config.is_some() || patch.max_points.is_some() {
            let kind = current.assignment_type;
            let config = patch.config.as_ref().unwrap_or(&current.config);
            if patch.config.is_some() {
                kind.validate_config(config, rules.profiles)?;
                files.extend(
                    kind.config_file_refs(config)?
                        .into_iter()
                        .map(|file| FileRef {
                            path: format!("config.{}", file.path),
                            ..file
                        }),
                );
            }
            check_points_total(kind, config, patch.max_points.unwrap_or(current.max_points))?;
            // The structure is frozen once work was submitted (ADR-0023); the assignment row is
            // locked, so no submit slips in between the flow's check and the write.
            changes_structure =
                patch.config.is_some() && kind.structure(&current.config) != kind.structure(config);
        }
        if points_change {
            let points = patch.max_points.unwrap_or(current.max_points);
            if points.value() < current.max_points.value() {
                lowered_to = Some(points);
            } else if points.value() > current.max_points.value() {
                raised_to = Some(points);
            }
        }
    }
    Ok(ItemUpdate {
        id,
        course_id,
        access,
        scale: rules.scale,
        unchanged: None,
        patch,
        is_assignment,
        body,
        settings,
        was_due,
        changes_structure,
        lowered_to,
        raised_to,
        files,
    })
}

/// Second half: refuses what the check of the files said (a file the item may not name, an image
/// block that names no image), then what the facts of the other modules forbid (a changed
/// structure after work was submitted, points below a score already given, points the grading
/// scale does not allow), then writes the update.
pub async fn finish_update_item(
    conn: &mut PgConnection,
    actor: &Actor,
    update: ItemUpdate,
    facts: AssignmentFacts,
    files: &FileVerdict,
) -> Result<Item, ContentError> {
    let ItemUpdate {
        id,
        course_id,
        access,
        scale,
        unchanged,
        patch,
        is_assignment,
        body,
        settings,
        was_due,
        changes_structure,
        lowered_to,
        raised_to,
        files: refs,
    } = update;
    if let Some(mut item) = unchanged {
        item.access = node_access(conn, actor, access, course_id, Node::Item(id)).await?;
        return Ok(item);
    }
    judge_files(&refs, files)?;
    if changes_structure && facts.has_submissions {
        return Err(ContentError::StructureFrozen);
    }
    if let Some(points) = lowered_to
        && facts
            .highest_score
            .is_some_and(|highest| points.value() < highest)
    {
        return Err(ContentError::BelowGivenScores);
    }
    if let Some(points) = raised_to {
        check_points(scale, points.value(), "max_points")?;
    }
    let mut item = repo::update_item(
        conn,
        id,
        patch.title.as_ref().map(ContentTitle::as_str),
        body.as_ref(),
        patch.publish_at.is_some(),
        patch.publish_at.flatten(),
    )
    .await?
    .ok_or(ContentError::ItemNotFound)?;
    item.assignment = if is_assignment && patch.touches_assignment() {
        Some(
            repo::update_assignment(
                conn,
                id,
                patch.config.as_ref(),
                patch.max_points,
                patch.is_bonus,
                settings,
                patch.class_on,
            )
            .await?
            .ok_or(ContentError::ItemNotFound)?,
        )
    } else if is_assignment {
        repo::find_assignment(conn, id).await?
    } else {
        None
    };
    if body.is_some() || patch.config.is_some() {
        sync_working_files(conn, &item).await?;
    }
    if patch.config.is_some() && item.published.is_some() {
        // Students read the config as it is, so a published item's images follow it at once.
        sync_published_files(conn, &item).await?;
    }
    // Students see an assignment that is published; a change to a draft is not news (ADR-0075).
    if item.status == ItemStatus::Published && is_assignment && patch.touches_assignment() {
        let (course_id, assignment_id) = (item.course_id, id);
        events::emit(
            conn,
            Event::AssignmentUpdated {
                course_id,
                assignment_id,
            },
        )
        .await?;
        let now_due = item.assignment.as_ref().and_then(|a| a.settings.due_at);
        if was_due.is_some_and(|before| before != now_due) {
            events::emit(
                conn,
                Event::AssignmentDeadlineChanged {
                    course_id,
                    assignment_id,
                },
            )
            .await?;
        }
    }
    if let Some(inputs) = &patch.access_rule {
        let tree = load_tree(conn, actor, course_id, access).await?;
        standing::set_rule(conn, course_id, Node::Item(id), inputs, &tree).await?;
    }
    audit(conn, actor, "content.item.update", "item", id, course_id).await?;
    item.access = node_access(conn, actor, access, course_id, Node::Item(id)).await?;
    Ok(item)
}

/// Sets (`Some`) or clears (`None`) the deadline of an assignment. A deadline comes with `on_deadline`
/// and `accept_late`, so an assignment that has none yet gets the editor's own defaults (keep the
/// draft, no late work) and one that has them keeps what was chosen; clearing the deadline clears
/// the three with `late_until`. It is `update_item` for what a module (ADR-0070) can change without
/// the configuration: the patch touches neither points nor a document, so the grading scale and the
/// editor profiles that `update_item` reads for those are never consulted.
pub async fn set_due_at(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
    due_at: Option<DateTime<Utc>>,
) -> Result<Item, ContentError> {
    let item = get_item(conn, actor, id).await?.item;
    let current = item.assignment.as_ref().map(|a| &a.settings);
    let mut settings = SettingsPatch {
        due_at: Some(due_at),
        ..SettingsPatch::default()
    };
    if due_at.is_some() {
        if current.is_none_or(|s| s.on_deadline.is_none()) {
            settings.on_deadline = Some(Some(OnDeadline::KeepDraft.as_str().to_owned()));
        }
        if current.is_none_or(|s| s.accept_late.is_none()) {
            settings.accept_late = Some(Some(false));
        }
    } else {
        settings.on_deadline = Some(None);
        settings.accept_late = Some(None);
        settings.late_until = Some(None);
    }
    let rules = ItemRules {
        scale: GradingScale::Twelve,
        profiles: &EffectiveProfiles::unrestricted(),
    };
    let patch = ItemPatch {
        settings,
        ..ItemPatch::default()
    };
    // A deadline touches neither points nor config, so nothing is asked of the modules above.
    let update = begin_update_item(conn, actor, rules, id, patch).await?;
    finish_update_item(
        conn,
        actor,
        update,
        AssignmentFacts::default(),
        &FileVerdict::default(),
    )
    .await
}

/// An item the actor may delete, with its assignment row locked if it is an assignment: the state
/// between [`begin_delete_item`] and [`finish_delete_item`].
#[derive(Debug)]
pub struct ItemDeletion {
    id: Uuid,
    module_id: Uuid,
    course_id: Uuid,
    is_assignment: bool,
}

impl ItemDeletion {
    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Only an assignment can have grades and attempts that the delete must respect.
    pub fn is_assignment(&self) -> bool {
        self.is_assignment
    }
}

/// First half of deleting a `text` item or an assignment: authorizes, then locks the assignment
/// row first, so the checks of the flow (`learning::items::delete_item`), in later statements, see
/// what a grade write or a submit in flight committed (see [`begin_delete_module`]).
pub async fn begin_delete_item(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<ItemDeletion, ContentError> {
    let (row, _) = manage_item(conn, actor, id).await?;
    let is_assignment = row.item.kind == ItemKind::Assignment;
    if is_assignment {
        repo::lock_assignment(conn, id).await?;
    }
    Ok(ItemDeletion {
        id,
        module_id: row.item.module_id,
        course_id: row.item.course_id,
        is_assignment,
    })
}

/// Second half: deletes the item and closes the gap in its module.
pub async fn finish_delete_item(
    conn: &mut PgConnection,
    actor: &Actor,
    deletion: ItemDeletion,
) -> Result<(), ContentError> {
    let ItemDeletion {
        id,
        module_id,
        course_id,
        ..
    } = deletion;
    if repo::delete_item(conn, id).await? == 0 {
        return Err(ContentError::ItemNotFound);
    }
    repo::compact_items(conn, module_id).await?;
    audit(conn, actor, "content.item.delete", "item", id, course_id).await?;
    Ok(())
}

/// Reorders the items of a module. `ids` must be the complete current list.
pub async fn reorder_items(
    conn: &mut PgConnection,
    actor: &Actor,
    module_id: Uuid,
    ids: &[Uuid],
) -> Result<(), ContentError> {
    let (row, _) = manage_module(conn, actor, module_id).await?;
    let current = repo::item_ids(conn, module_id).await?;
    check_order(ids, &current)?;
    let rewritten = repo::reorder_items(conn, module_id, ids).await?;
    check_rewritten(rewritten, ids)?;
    audit(
        conn,
        actor,
        "content.item.reorder",
        "module",
        module_id,
        row.module.course_id,
    )
    .await?;
    Ok(())
}

/// Publishes an item. Publishing a published item changes nothing; `publish_at` is not touched.
pub async fn publish_item(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<Item, ContentError> {
    let (row, access) = manage_item(conn, actor, id).await?;
    let mut item = match repo::publish_item(conn, id).await? {
        Some(item) => {
            sync_published_files(conn, &item).await?;
            announce_visible(conn, Utc::now(), Some(item.course_id)).await?;
            let item = with_assignment(conn, item).await?;
            audit(
                conn,
                actor,
                "content.item.publish",
                "item",
                id,
                item.course_id,
            )
            .await?;
            item
        }
        None => {
            // Already published, or deleted while this request waited.
            if repo::find_item(conn, id).await?.is_none() {
                return Err(ContentError::ItemNotFound);
            }
            row.item
        }
    };
    item.access = node_access(conn, actor, access, item.course_id, Node::Item(id)).await?;
    Ok(item)
}

/// The student marks a text or file item done (ADR-0092). Marking it again changes nothing. An
/// assignment takes no mark, it is done by what the student submits or is graded, and an item that
/// a rule still keeps closed cannot be marked, because the student has not read it.
pub async fn mark_done(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<(), ContentError> {
    let (row, access) = open_item(conn, actor, id).await?;
    access::authorize(actor, Action::MarkDone(access))?;
    if row.item.kind == ItemKind::Assignment {
        return Err(ContentError::CompletesItself);
    }
    if is_locked(conn, actor, access, row.item.course_id, id).await? {
        return Err(ContentError::Locked);
    }
    repo::mark_done(conn, id, actor.user_id()).await?;
    Ok(())
}

/// The student takes their mark off an item. With no mark it does nothing. A locked item can lose
/// its mark: the rule changed after the student marked it.
pub async fn unmark_done(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<(), ContentError> {
    let (_, access) = open_item(conn, actor, id).await?;
    access::authorize(actor, Action::MarkDone(access))?;
    repo::unmark_done(conn, id, actor.user_id()).await?;
    Ok(())
}

/// Holds the facts of `scope` until the end of the transaction. A flow that copies the facts from
/// their sources calls it first and reads the sources after, so a concurrent change it waited for
/// is in what it reads (ADR-0093). It authorizes nothing: the flow acts on an event, not a request.
pub async fn lock_facts(conn: &mut PgConnection, scope: &FactScope) -> Result<(), ContentError> {
    repo::lock_facts(conn, scope).await?;
    Ok(())
}

/// Makes the submitted-and-scored part of the facts inside `scope` equal to `rows`, which come
/// from `assessment` and `gradebook`: every row of the scope is forgotten first, then the rows
/// that say something are written. The student's own marks stay. It authorizes nothing, like
/// [`lock_facts`].
pub async fn replace_source_facts(
    conn: &mut PgConnection,
    scope: &FactScope,
    rows: &[SourceFact],
) -> Result<(), ContentError> {
    repo::clear_source_facts(conn, scope).await?;
    let saying_something: Vec<SourceFact> = rows
        .iter()
        .filter(|row| row.submitted_at.is_some() || row.published_score.is_some())
        .cloned()
        .collect();
    repo::upsert_source_facts(conn, &saying_something).await?;
    Ok(())
}

#[cfg(test)]
mod tests;
