//! Business rules. Every function that acts for a user authorizes first, takes the connection it
//! runs on and never begins or commits a transaction.
//!
//! A module or item the actor may not view does not exist as far as they can tell (404): an
//! unknown id, an id in a course they cannot see and a draft they may not open all give the same
//! error. Only after that does `EditContent` decide between going ahead and 403.

use std::collections::{HashMap, HashSet};

use api_types::ProfileName;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::PgConnection;
use sqlx::types::Decimal;
use uuid::Uuid;

use super::document::{DocumentErrors, EffectiveProfiles, ValidDocument, validate_document};
use super::model::{
    Assignment, AssignmentType, ContentModule, ContentSubModule, ContentTitle, Item, ItemKind,
    ItemPatch, ModelError, Module, ModulePatch, OpenedAssignment, SettingProblem, editor_features,
};
use super::repo::{self, ItemRow, ModuleRow};
use crate::actor::Actor;
use crate::assessment::{self, AssessmentError};
use crate::events::{self, AuditEntry, EventsError};
use crate::gradebook::{self, GradebookError};
use crate::policy::{self, Action, CourseAccess, PolicyError, Release};
use crate::settings::GradingScale;
use crate::teaching::{self, TeachingError};

/// The most points one assignment can be worth on the `twelve` scale (ADR-0022).
const TWELVE_LIMIT: u32 = 12;
/// The most points the regular assignments of a course can add up to on the `hundred` scale.
const HUNDRED_LIMIT: u32 = 100;

#[derive(Debug, thiserror::Error)]
pub enum ContentError {
    #[error(transparent)]
    Forbidden(#[from] PolicyError),
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
    /// No such module, or the actor may not see it.
    #[error("module not found")]
    ModuleNotFound,
    /// No such item, or the actor may not see it.
    #[error("item not found")]
    ItemNotFound,
    /// A concurrent change got in the way (a position taken, a parent deleted). Retry.
    #[error("a concurrent change conflicts with this one")]
    Conflict,
    /// The settings break a rule once the change is applied (ADR-0035); one entry per field.
    #[error("the settings are invalid")]
    Settings(Vec<SettingProblem>),
    /// `max_points` would drop below a score already given (ADR-0021, ADR-0028).
    #[error("max_points cannot be lowered below a score that was already given")]
    BelowGivenScores,
    /// The assignment, or an assignment below the module, has a grade (ADR-0028).
    #[error("it has grades: clear them before deleting")]
    HasGrades,
    /// The assignment, or an assignment below the module, has a submitted attempt (ADR-0035).
    #[error("it has submitted work")]
    HasSubmissions,
    #[error(transparent)]
    Gradebook(GradebookError),
    #[error(transparent)]
    Assessment(AssessmentError),
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

impl From<GradebookError> for ContentError {
    fn from(error: GradebookError) -> Self {
        match error {
            GradebookError::HasGrades => Self::HasGrades,
            other => Self::Gradebook(other),
        }
    }
}

impl From<AssessmentError> for ContentError {
    fn from(error: AssessmentError) -> Self {
        match error {
            AssessmentError::HasSubmissions => Self::HasSubmissions,
            other => Self::Assessment(other),
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
    policy::authorize(actor, view).map_err(|_| ContentError::ModuleNotFound)?;
    Ok((row, access))
}

/// Loads a module the actor may view and change.
async fn manage_module(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<ModuleRow, ContentError> {
    let (row, access) = open_module(conn, actor, id).await?;
    policy::authorize(actor, Action::EditContent(access))?;
    Ok(row)
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
    policy::authorize(actor, view).map_err(|_| ContentError::ItemNotFound)?;
    Ok((row, access))
}

/// Loads an item the actor may view and change.
async fn manage_item(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<ItemRow, ContentError> {
    let (row, access) = open_item(conn, actor, id).await?;
    policy::authorize(actor, Action::EditContent(access))?;
    Ok(row)
}

/// The course access of an actor who wants to change the course's content.
async fn manage_course(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<CourseAccess, ContentError> {
    let access = teaching::course_access(conn, actor, course_id).await?;
    policy::authorize(actor, Action::EditContent(access))?;
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
/// change needs the check at all: creating, raising `max_points` and turning a bonus assignment
/// into a regular one do; lowering, deleting and turning a regular one into a bonus never do.
///
/// `hundred`: takes the course's points lock, then reads the sum in a later statement, so a
/// writer that waited for the lock sees the rows the holder committed. `own` is the assignment
/// being changed: its current points are not part of the sum it is compared with.
async fn check_points(
    conn: &mut PgConnection,
    scale: GradingScale,
    course_id: Uuid,
    own: Option<Uuid>,
    points: Decimal,
    is_bonus: bool,
    field: &'static str,
) -> Result<(), ContentError> {
    match scale {
        GradingScale::Twelve => {
            if points > Decimal::from(TWELVE_LIMIT) {
                return Err(ContentError::PointsLimit {
                    field,
                    message: format!(
                        "an assignment may be worth at most {TWELVE_LIMIT} points on the twelve-point scale"
                    ),
                });
            }
        }
        GradingScale::Hundred => {
            if is_bonus {
                return Ok(());
            }
            repo::lock_course_points(conn, course_id).await?;
            let used = repo::regular_points(conn, course_id, own).await?;
            let limit = Decimal::from(HUNDRED_LIMIT);
            if used + points > limit {
                let free = (limit - used).max(Decimal::ZERO);
                return Err(ContentError::PointsLimit {
                    field,
                    message: format!(
                        "the regular assignments of a course may total at most {HUNDRED_LIMIT} points; {free:.2} are still free"
                    ),
                });
            }
        }
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
    let sees_drafts = policy::authorize(actor, Action::ManageContent(access)).is_ok();
    let (total_points, graded_items, changed_at) =
        repo::course_summary(conn, course_id, (!sees_drafts).then(Utc::now)).await?;
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
/// has authorized both courses.
pub async fn copy_course_content(
    conn: &mut PgConnection,
    from: Uuid,
    to: Uuid,
) -> Result<(), ContentError> {
    repo::copy_course(conn, from, to).await?;
    Ok(())
}

/// The course's content as the actor may see it. Staff see everything; others only what is open
/// to students now. Not paged: a course tree is bounded (ADR-0020).
pub async fn get_tree(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<Vec<ContentModule>, ContentError> {
    let access = teaching::course_access(conn, actor, course_id).await?;
    let now = Utc::now();
    let sees_drafts = policy::authorize(actor, Action::ManageContent(access)).is_ok();
    let rows = repo::tree(conn, course_id, (!sees_drafts).then_some(now)).await?;

    let mut top_level = Vec::new();
    let mut submodules_of: HashMap<Uuid, Vec<Module>> = HashMap::new();
    let mut items_of = HashMap::new();
    let mut seen = HashSet::new();
    for row in rows {
        let path = Path::new(row.module.release(), row.parent);
        if policy::authorize(actor, path.view(access, None, now)).is_err() {
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
            && policy::authorize(actor, path.view(access, Some(item.release()), now)).is_ok()
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
    let row = manage_module(conn, actor, id).await?;
    if patch.is_empty() {
        return Ok(row.module);
    }
    let module = repo::update_module(
        conn,
        id,
        patch.title.as_ref().map(ContentTitle::as_str),
        patch.publish_at.is_some(),
        patch.publish_at.flatten(),
    )
    .await?
    // Deleted while this request waited for the row: it is gone, not a failure (ADR-0039).
    .ok_or(ContentError::ModuleNotFound)?;
    audit(
        conn,
        actor,
        "content.module.update",
        "module",
        id,
        module.course_id,
    )
    .await?;
    Ok(module)
}

/// Deletes the module with its sub-modules and items.
pub async fn delete_module(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<(), ContentError> {
    let row = manage_module(conn, actor, id).await?;
    // Lock every assignment below first: a grade write in flight holds one of them, so the check
    // that follows, in a later statement, sees its grade (ADR-0028). The empty rows that are left
    // go with their history; the grades table has no foreign key to cascade through.
    repo::lock_assignments_below(conn, id).await?;
    if gradebook::has_grades_in_module(conn, id).await? {
        return Err(ContentError::HasGrades);
    }
    // A submit holds the same row `FOR SHARE`, so an attempt in flight is committed by now
    // (ADR-0035). Drafts without an attempt go with the module.
    if assessment::has_submitted_in_module(conn, id).await? {
        return Err(ContentError::HasSubmissions);
    }
    gradebook::purge_module(conn, id).await?;
    assessment::purge_module(conn, id).await?;
    // A delete that lost the race to another one found nothing: no answer of success and no
    // audit row for something that did not happen (ADR-0039).
    if repo::delete_module(conn, id).await? == 0 {
        return Err(ContentError::ModuleNotFound);
    }
    repo::compact_modules(conn, row.module.course_id, row.module.parent_id).await?;
    audit(
        conn,
        actor,
        "content.module.delete",
        "module",
        id,
        row.module.course_id,
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
    let row = manage_module(conn, actor, id).await?;
    let published = repo::publish_module(conn, id).await?;
    if published.is_none() && repo::find_module(conn, id).await?.is_none() {
        return Err(ContentError::ModuleNotFound);
    }
    let mut changed = published.is_some();
    if cascade {
        let submodules = repo::publish_draft_submodules(conn, id).await?;
        let items = repo::publish_draft_items_below(conn, id).await?;
        changed |= submodules + items > 0;
    }
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
    Ok(published.unwrap_or(row.module))
}

// ---------------------------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------------------------

/// What the item writes read from the configuration: the grading scale (ADR-0022) and the
/// resolved editor profiles (ADR-0025). Both are resolved once, when `AppState` is built.
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

/// Creates a draft item as the last of the module, which may be a top-level module or a
/// sub-module. An `assignment` makes it an item of kind `assignment`; without one it is a `text` item.
pub async fn create_item(
    conn: &mut PgConnection,
    actor: &Actor,
    rules: ItemRules<'_>,
    module_id: Uuid,
    title: ContentTitle,
    body: Option<Value>,
    assignment: Option<Assignment>,
) -> Result<Item, ContentError> {
    let row = manage_module(conn, actor, module_id).await?;
    let course_id = row.module.course_id;
    if let Some(assignment) = &assignment {
        assignment
            .assignment_type
            .validate_config(&assignment.config)?;
        check_editor_features(&assignment.config, rules.profiles)?;
    }
    let body = document(body.as_ref(), rules.profiles)?;
    if let Some(assignment) = &assignment {
        check_points(
            conn,
            rules.scale,
            course_id,
            None,
            assignment.max_points.value(),
            assignment.is_bonus,
            "max_points",
        )
        .await?;
    }
    let kind = if assignment.is_some() {
        ItemKind::Assignment
    } else {
        ItemKind::Text
    };
    let mut item = repo::insert_item(
        conn,
        Uuid::now_v7(),
        module_id,
        course_id,
        kind,
        title.as_str(),
        body.unwrap_or_else(ValidDocument::empty).as_body(),
    )
    .await?;
    if let Some(assignment) = &assignment {
        item.assignment = Some(repo::insert_assignment(conn, item.id, assignment).await?);
    }
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

/// The profiles the document editor enforces, for building its toolbar. Any role may ask.
pub fn editor_profiles<'a>(
    actor: &Actor,
    profiles: &'a EffectiveProfiles,
) -> Result<&'a EffectiveProfiles, ContentError> {
    policy::authorize(actor, Action::ViewEditorProfiles)?;
    Ok(profiles)
}

pub async fn get_item(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<Item, ContentError> {
    Ok(open_item(conn, actor, id).await?.0.item)
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
    policy::authorize(actor, view).map_err(|_| OpenError::NotFound)?;
    Ok(OpenedAssignment {
        course_id: row.course_id,
        assignment_type: row.assignment_type,
        access,
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

/// `config.editor.features` can only narrow the student profile (ADR-0025): a feature it lists
/// that the effective profile, after the institution's configuration, does not have is refused.
fn check_editor_features(
    config: &serde_json::Map<String, Value>,
    profiles: &EffectiveProfiles,
) -> Result<(), ModelError> {
    let allowed = &profiles.profile(ProfileName::Student).features;
    for (index, feature) in editor_features(config)?.into_iter().flatten().enumerate() {
        if !allowed.contains(&feature) {
            return Err(ModelError::Config(format!(
                "`editor.features[{index}]` is not a feature of the student profile"
            )));
        }
    }
    Ok(())
}

/// Changes the title, body and/or `publish_at`, and for an assignment its `config`, `max_points`
/// and `is_bonus`. A change with no fields writes nothing.
///
/// The row loaded to authorize may be stale by the time the points lock is granted (READ COMMITTED
/// lets another writer commit while this one waits). So whether `max_points` is raised, and
/// whether a bonus becomes regular, is decided on the assignment re-read after the lock is held.
pub async fn update_item(
    conn: &mut PgConnection,
    actor: &Actor,
    rules: ItemRules<'_>,
    id: Uuid,
    patch: ItemPatch,
) -> Result<Item, ContentError> {
    let row = manage_item(conn, actor, id).await?;
    if patch.is_empty() {
        return Ok(row.item);
    }
    let is_assignment = row.item.kind == ItemKind::Assignment;
    if !is_assignment && patch.touches_assignment() {
        let field = if patch.config.is_some() {
            "config"
        } else if patch.max_points.is_some() {
            "max_points"
        } else if patch.is_bonus.is_some() {
            "is_bonus"
        } else {
            patch.settings.first_field()
        };
        return Err(ContentError::NotAnAssignment { field });
    }
    if let (Some(config), Some(assignment)) = (&patch.config, &row.item.assignment) {
        assignment.assignment_type.validate_config(config)?;
        check_editor_features(config, rules.profiles)?;
    }
    let body = document(patch.body.as_ref(), rules.profiles)?;
    let mut settings = None;
    if is_assignment && patch.touches_assignment() {
        let points_change = patch.max_points.is_some() || patch.is_bonus.is_some();
        if points_change && rules.scale == GradingScale::Hundred {
            repo::lock_course_points(conn, row.item.course_id).await?;
        }
        // The row lock comes before the gradebook is asked, which is a later statement: a grade
        // write holds the row `FOR SHARE`, so this waits for it and then sees its score. A
        // submission write holds it the same way, so a submit that began first finishes under the
        // old settings and one that waits sees the new ones (ADR-0035). The current values are
        // the ones in the locked row.
        let current = repo::lock_assignment(conn, id)
            .await?
            .ok_or(ContentError::ItemNotFound)?;
        if !patch.settings.is_empty() {
            settings = Some(
                current
                    .settings
                    .resolve(&patch.settings, current.assignment_type.takes_submissions())
                    .map_err(ContentError::Settings)?,
            );
        }
        if points_change {
            let points = patch.max_points.unwrap_or(current.max_points);
            let is_bonus = patch.is_bonus.unwrap_or(current.is_bonus);
            if points.value() < current.max_points.value()
                && gradebook::max_score(conn, id)
                    .await?
                    .is_some_and(|highest| points.value() < highest)
            {
                return Err(ContentError::BelowGivenScores);
            }
            let raised = points.value() > current.max_points.value();
            let becomes_regular = current.is_bonus && !is_bonus;
            let needs_check = match rules.scale {
                GradingScale::Twelve => raised,
                GradingScale::Hundred => raised || becomes_regular,
            };
            if needs_check {
                let field = if raised { "max_points" } else { "is_bonus" };
                check_points(
                    conn,
                    rules.scale,
                    row.item.course_id,
                    Some(id),
                    points.value(),
                    is_bonus,
                    field,
                )
                .await?;
            }
        }
    }
    let mut item = repo::update_item(
        conn,
        id,
        patch.title.as_ref().map(ContentTitle::as_str),
        body.as_ref().map(ValidDocument::as_body),
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
            )
            .await?
            .ok_or(ContentError::ItemNotFound)?,
        )
    } else if is_assignment {
        repo::find_assignment(conn, id).await?
    } else {
        None
    };
    audit(
        conn,
        actor,
        "content.item.update",
        "item",
        id,
        item.course_id,
    )
    .await?;
    Ok(item)
}

pub async fn delete_item(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<(), ContentError> {
    let row = manage_item(conn, actor, id).await?;
    if row.item.kind == ItemKind::Assignment {
        // The row lock first, then the grades in a later statement (see `delete_module`).
        repo::lock_assignment(conn, id).await?;
        if gradebook::has_grades_for_assignment(conn, id).await? {
            return Err(ContentError::HasGrades);
        }
        if assessment::has_submitted_for_assignment(conn, id).await? {
            return Err(ContentError::HasSubmissions);
        }
        gradebook::purge_assignment(conn, id).await?;
        assessment::purge_assignment(conn, id).await?;
    }
    if repo::delete_item(conn, id).await? == 0 {
        return Err(ContentError::ItemNotFound);
    }
    repo::compact_items(conn, row.item.module_id).await?;
    audit(
        conn,
        actor,
        "content.item.delete",
        "item",
        id,
        row.item.course_id,
    )
    .await?;
    Ok(())
}

/// Reorders the items of a module. `ids` must be the complete current list.
pub async fn reorder_items(
    conn: &mut PgConnection,
    actor: &Actor,
    module_id: Uuid,
    ids: &[Uuid],
) -> Result<(), ContentError> {
    let row = manage_module(conn, actor, module_id).await?;
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
    let row = manage_item(conn, actor, id).await?;
    match repo::publish_item(conn, id).await? {
        Some(item) => {
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
            Ok(item)
        }
        None => {
            // Already published, or deleted while this request waited.
            if repo::find_item(conn, id).await?.is_none() {
                return Err(ContentError::ItemNotFound);
            }
            Ok(row.item)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::db_test;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use serde_json::Map;

    use std::sync::LazyLock;

    use super::*;
    use crate::error::ApiError;
    use crate::identity::{self, Email, NewUser, PersonName, Role};
    use crate::teaching::CourseTitle;

    static PROFILES: LazyLock<EffectiveProfiles> = LazyLock::new(EffectiveProfiles::unrestricted);

    /// The `hundred` scale and the profiles of the code, as most tests want them.
    fn rules() -> ItemRules<'static> {
        ItemRules {
            scale: GradingScale::Hundred,
            profiles: &PROFILES,
        }
    }

    /// Two creators each read "the last position" before the other commits, so both pick the
    /// same one. The unique constraint is deferred: neither insert waits for the other, and the
    /// loser fails at its own commit. The steps run one after another on purpose, so the
    /// interleaving is fixed by the code, not by timing.
    #[tokio::test]
    async fn two_creates_racing_for_the_last_position_one_gets_409() {
        db_test(async |pool| {
            let hash = identity::hash_password("irrelevant").await.unwrap();
            let mut tx = pool.begin().await.unwrap();
            let user = identity::create_user_unaudited_for_tests(
                &mut tx,
                NewUser {
                    email: Email::parse("owner@x.edu").unwrap(),
                    name: PersonName::name("Owner").unwrap(),
                    surname: PersonName::surname("Owner").unwrap(),
                    role: Role::Teacher,
                },
                hash,
            )
            .await
            .unwrap();
            let owner = Actor::new(user.id, Role::Teacher, Uuid::now_v7());
            let course = teaching::create_course(
                &mut tx,
                &owner,
                CourseTitle::parse("Algebra").unwrap(),
                None,
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();

            let title = |text| ContentTitle::parse(text).unwrap();
            let mut winner = pool.begin().await.unwrap();
            let mut loser = pool.begin().await.unwrap();
            let first = create_module(&mut winner, &owner, course.id, title("First"), None)
                .await
                .unwrap();
            let second = create_module(&mut loser, &owner, course.id, title("Second"), None)
                .await
                .unwrap();
            assert_eq!((first.position, second.position), (1, 1));

            winner.commit().await.unwrap();
            let error =
                ContentError::from(loser.commit().await.expect_err("the position is taken"));
            assert!(matches!(error, ContentError::Conflict), "{error:?}");
            assert_eq!(
                ApiError::from(error).into_response().status(),
                StatusCode::CONFLICT
            );

            let mut conn = pool.acquire().await.unwrap();
            let ids = repo::module_ids(&mut conn, course.id, None).await.unwrap();
            assert_eq!(ids, [first.id], "only the winner is stored");

            // The client retries and now appends after the winner.
            let mut retry = pool.begin().await.unwrap();
            let again = create_module(&mut retry, &owner, course.id, title("Second"), None)
                .await
                .unwrap();
            retry.commit().await.unwrap();
            assert_eq!(again.position, 2);
        })
        .await;
    }

    // -----------------------------------------------------------------------------------------
    // The points lock (ADR-0022). Each test holds the lock in one transaction, starts a second
    // writer, waits until that writer is observed blocked on the lock, and only then lets the
    // first one commit. Nothing sleeps: the order is forced by the lock, not by timing.
    // -----------------------------------------------------------------------------------------

    use std::time::Duration;

    use crate::content::model::{BuiltinType, MaxPoints};

    fn text(points: &str, is_bonus: bool) -> Assignment {
        Assignment {
            assignment_type: BuiltinType::Text,
            config: Map::new(),
            max_points: MaxPoints::parse(points).unwrap(),
            is_bonus,
            settings: Default::default(),
        }
    }

    fn points(text: &str) -> ItemPatch {
        ItemPatch {
            max_points: Some(MaxPoints::parse(text).unwrap()),
            ..ItemPatch::default()
        }
    }

    fn bonus(is_bonus: bool) -> ItemPatch {
        ItemPatch {
            is_bonus: Some(is_bonus),
            ..ItemPatch::default()
        }
    }

    struct Course {
        pool: sqlx::PgPool,
        owner: Actor,
        module: Uuid,
    }

    async fn course(pool: sqlx::PgPool) -> Course {
        let hash = identity::hash_password("irrelevant").await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        let user = identity::create_user_unaudited_for_tests(
            &mut tx,
            NewUser {
                email: Email::parse("owner@x.edu").unwrap(),
                name: PersonName::name("Owner").unwrap(),
                surname: PersonName::surname("Owner").unwrap(),
                role: Role::Teacher,
            },
            hash,
        )
        .await
        .unwrap();
        let owner = Actor::new(user.id, Role::Teacher, Uuid::now_v7());
        let course = teaching::create_course(
            &mut tx,
            &owner,
            CourseTitle::parse("Algebra").unwrap(),
            None,
        )
        .await
        .unwrap();
        let module = create_module(
            &mut tx,
            &owner,
            course.id,
            ContentTitle::parse("M").unwrap(),
            None,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        Course {
            pool,
            owner,
            module: module.id,
        }
    }

    impl Course {
        /// Creates an assignment in the given transaction on the `hundred` scale.
        async fn create(
            &self,
            conn: &mut PgConnection,
            points: &str,
            is_bonus: bool,
        ) -> Result<Item, ContentError> {
            create_item(
                conn,
                &self.owner,
                rules(),
                self.module,
                ContentTitle::parse("A").unwrap(),
                None,
                Some(text(points, is_bonus)),
            )
            .await
        }

        /// Creates and commits an assignment.
        async fn committed(&self, points: &str, is_bonus: bool) -> Uuid {
            let mut tx = self.pool.begin().await.unwrap();
            let item = self.create(&mut tx, points, is_bonus).await.unwrap();
            tx.commit().await.unwrap();
            item.id
        }

        async fn patch(
            &self,
            conn: &mut PgConnection,
            id: Uuid,
            patch: ItemPatch,
        ) -> Result<Item, ContentError> {
            update_item(conn, &self.owner, rules(), id, patch).await
        }

        /// Runs `patch` in its own transaction on a task and commits it when it succeeds.
        fn patch_in_background(
            &self,
            id: Uuid,
            patch: ItemPatch,
        ) -> tokio::task::JoinHandle<Result<Item, ContentError>> {
            let (pool, owner) = (self.pool.clone(), self.owner.clone());
            tokio::spawn(async move {
                let mut tx = pool.begin().await.unwrap();
                let result = update_item(&mut tx, &owner, rules(), id, patch).await;
                if result.is_ok() {
                    tx.commit().await.unwrap();
                }
                result
            })
        }

        async fn assignment(&self, id: Uuid) -> Assignment {
            let mut conn = self.pool.acquire().await.unwrap();
            repo::find_assignment(&mut conn, id).await.unwrap().unwrap()
        }

        async fn regular_points(&self) -> String {
            let mut conn = self.pool.acquire().await.unwrap();
            let course_id = repo::find_module(&mut conn, self.module)
                .await
                .unwrap()
                .unwrap()
                .module
                .course_id;
            repo::regular_points(&mut conn, course_id, None)
                .await
                .unwrap()
                .to_string()
        }
    }

    /// Returns once a session waits for an advisory lock. Fails if the writer finished instead
    /// (it did not wait for the lock) or if nobody waits within the timeout.
    async fn wait_until_blocked<T>(pool: &sqlx::PgPool, writer: &tokio::task::JoinHandle<T>) {
        let mut conn = pool.acquire().await.unwrap();
        let waited = tokio::time::timeout(Duration::from_secs(10), async {
            while !writer.is_finished() && repo::advisory_lock_waiters(&mut conn).await.unwrap() < 1
            {
                tokio::task::yield_now().await;
            }
        })
        .await;
        assert!(waited.is_ok(), "no writer waited for the points lock");
        assert!(
            !writer.is_finished(),
            "the second writer finished without waiting for the points lock"
        );
    }

    fn points_limit(result: Result<Item, ContentError>) -> (&'static str, String) {
        match result {
            Err(ContentError::PointsLimit { field, message }) => (field, message),
            other => panic!("expected the points limit, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn two_creates_of_sixty_regular_points_one_gets_422() {
        db_test(async |pool| {
            let course = course(pool).await;

            let mut first = course.pool.begin().await.unwrap();
            course.create(&mut first, "60", false).await.unwrap();

            let second = tokio::spawn({
                let (pool, owner, module) =
                    (course.pool.clone(), course.owner.clone(), course.module);
                async move {
                    let mut tx = pool.begin().await.unwrap();
                    let result = create_item(
                        &mut tx,
                        &owner,
                        rules(),
                        module,
                        ContentTitle::parse("B").unwrap(),
                        None,
                        Some(text("60", false)),
                    )
                    .await;
                    if result.is_ok() {
                        tx.commit().await.unwrap();
                    }
                    result
                }
            });
            wait_until_blocked(&course.pool, &second).await;
            first.commit().await.unwrap();

            let error = second.await.unwrap();
            let (field, message) = points_limit(error);
            assert_eq!(field, "max_points");
            assert!(message.contains("40.00 are still free"), "{message}");
            assert_eq!(course.regular_points().await, "60.00");
        })
        .await;
    }

    #[tokio::test]
    async fn a_patch_that_looked_like_a_lowering_is_checked_again_after_the_wait() {
        db_test(async |pool| {
            let course = course(pool).await;
            let x = course.committed("40", false).await;
            let y = course.committed("60", false).await;
            assert_ne!(x, y);

            // The first writer lowers X to 10 and fills the freed points with a new assignment.
            let mut first = course.pool.begin().await.unwrap();
            course.patch(&mut first, x, points("10")).await.unwrap();
            course.create(&mut first, "30", false).await.unwrap();

            // The second writer loads X (40) to authorize, then waits for the lock. Against the 40 it
            // read, 30 is a lowering; against the 10 that is committed by then, it is a raise.
            let second = course.patch_in_background(x, points("30"));
            wait_until_blocked(&course.pool, &second).await;
            first.commit().await.unwrap();

            let (field, message) = points_limit(second.await.unwrap());
            assert_eq!(field, "max_points");
            assert!(message.contains("10.00 are still free"), "{message}");
            assert_eq!(course.assignment(x).await.max_points.to_string(), "10.00");
            assert_eq!(course.regular_points().await, "100.00");
        })
        .await;
    }

    #[tokio::test]
    async fn a_patch_that_looked_like_no_change_is_checked_again_after_the_wait() {
        db_test(async |pool| {
            let course = course(pool).await;
            let x = course.committed("50", false).await;
            course.committed("50", false).await;

            // The first writer makes X a bonus assignment and fills the freed points.
            let mut first = course.pool.begin().await.unwrap();
            course.patch(&mut first, x, bonus(true)).await.unwrap();
            course.create(&mut first, "50", false).await.unwrap();

            // The second writer loads X as a regular assignment, so `is_bonus = false` looks like no
            // change. Once the first commits, X is a bonus one and this makes it regular again.
            let second = course.patch_in_background(x, bonus(false));
            wait_until_blocked(&course.pool, &second).await;
            first.commit().await.unwrap();

            let (field, message) = points_limit(second.await.unwrap());
            assert_eq!(field, "is_bonus");
            assert!(message.contains("0.00 are still free"), "{message}");
            assert!(course.assignment(x).await.is_bonus);
            assert_eq!(course.regular_points().await, "100.00");
        })
        .await;
    }
}
