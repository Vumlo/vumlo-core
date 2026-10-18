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

use super::document::{
    DocumentErrors, EffectiveProfiles, ValidDocument, plain_text, validate_document,
};
use super::model::{
    Assignment, AssignmentType, BuiltinType, ContentModule, ContentSubModule, ContentTitle, Hit,
    HitKind, Item, ItemFacts, ItemKind, ItemPatch, ItemStatus, MaxPoints, ModelError, Module,
    ModulePatch, MyWork, OpenedAssignment, Revision, SettingProblem, WorkEntry, WorkFacts,
};
use super::repo::{self, ItemRow, ModuleRow};
use super::types::form;
use crate::actor::Actor;
use crate::assessment::{self, AssessmentError};
use crate::events::{self, AuditEntry, EventsError};
use crate::gradebook::{self, GradebookError};
use crate::policy::{self, Action, CourseAccess, CourseStatus, PolicyError, Release};
use crate::teaching::{self, TeachingError};
use vumlo_config::GradingScale;

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
    /// A revision can only be published for an item that is published (ADR-0049).
    #[error("the item is a draft: publish it first")]
    ItemNotPublished,
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
    /// The search text is not 2 to 100 characters (ADR-0052).
    #[error("the search text must be 2 to 100 characters")]
    BadQuery,
    /// The change touches the part of a quiz or form that is frozen once work was submitted
    /// (ADR-0023, ADR-0050, ADR-0051).
    #[error("the questions or fields cannot change after work was submitted")]
    StructureFrozen,
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
    mut assignment: Option<Assignment>,
) -> Result<Item, ContentError> {
    let row = manage_module(conn, actor, module_id).await?;
    let course_id = row.module.course_id;
    if let Some(assignment) = &mut assignment {
        let kind = assignment.assignment_type;
        kind.strip_computed(&mut assignment.config);
        kind.validate_config(&assignment.config, rules.profiles)?;
        check_points_total(kind, &assignment.config, assignment.max_points)?;
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

/// An item as the actor may read it: staff get the working copy, a student the published title and
/// body (ADR-0049).
pub async fn get_item(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<Item, ContentError> {
    let (row, access) = open_item(conn, actor, id).await?;
    let staff = policy::authorize(actor, Action::ManageContent(access)).is_ok();
    let mut item = if staff {
        row.item
    } else {
        row.item.as_seen_by_students()
    };
    if let Some(assignment) = &mut item.assignment {
        let kind = assignment.assignment_type;
        // A student never receives the answer key (ADR-0023, ADR-0050 point 6).
        if !staff {
            assignment.config = kind.student_config(&assignment.config);
        }
        if kind == BuiltinType::Form {
            let facts = assessment::form_facts(conn, id, kind, &assignment.config).await?;
            form::decorate(
                &mut assignment.config,
                &facts.answered,
                &facts.holders,
                staff,
                actor.user_id(),
            );
        }
    }
    Ok(item)
}

/// Whether a published item has changes students do not see, and since when they read what they
/// read. Staff only.
pub async fn get_revision(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<Revision, ContentError> {
    let (row, access) = open_item(conn, actor, id).await?;
    policy::authorize(actor, Action::ManageContent(access))?;
    Ok(Revision::of(&row.item))
}

/// Publishes the working title and body of a published item (ADR-0049). A draft has nothing to
/// publish a revision of: publish the item.
pub async fn publish_revision(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<Revision, ContentError> {
    let row = manage_item(conn, actor, id).await?;
    if row.item.status != ItemStatus::Published {
        return Err(ContentError::ItemNotPublished);
    }
    let item = repo::publish_revision(conn, id)
        .await?
        .ok_or(ContentError::ItemNotFound)?;
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

/// The facts of every item of the course that the actor may see (ADR-0049, point 5). Staff get
/// the counts of what to grade and which items have unpublished changes; a student of the course
/// gets their own standing. Not paged: a course tree is bounded (ADR-0020).
pub async fn item_meta(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<HashMap<Uuid, ItemFacts>, ContentError> {
    let access = teaching::course_access(conn, actor, course_id).await?;
    let staff = policy::authorize(actor, Action::ManageContent(access)).is_ok();
    let tree = get_tree(conn, actor, course_id).await?;
    let visible: HashSet<Uuid> = tree
        .iter()
        .flat_map(|top| {
            top.items
                .iter()
                .chain(top.submodules.iter().flat_map(|s| s.items.iter()))
        })
        .map(|item| item.id)
        .collect();
    let mut facts: HashMap<Uuid, ItemFacts> = visible
        .iter()
        .map(|id| (*id, ItemFacts::default()))
        .collect();
    for (id, body) in repo::lecture_bodies(conn, course_id, staff).await? {
        if let Some(entry) = facts.get_mut(&id) {
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
        for counts in repo::staff_counts(conn, course_id).await? {
            if let Some(entry) = facts.get_mut(&counts.assignment_id) {
                entry.staff = Some(counts);
            }
        }
    } else if access.is_student() {
        for work in repo::student_work(conn, actor.user_id(), &[course_id], Utc::now()).await? {
            if let Some(entry) = facts.get_mut(&work.assignment_id) {
                entry.work = Some(work);
            }
        }
    }
    Ok(facts)
}

/// Every assignment a student can open in their courses with their own standing on it, and the
/// courses (ADR-0052). Published values only. Not paged: a student has a bounded number of
/// courses and a course tree is bounded (ADR-0020).
pub async fn my_work(
    conn: &mut PgConnection,
    actor: &Actor,
    now: DateTime<Utc>,
) -> Result<MyWork, ContentError> {
    let courses = teaching::student_courses(conn, actor).await?;
    let ids: Vec<Uuid> = courses.iter().map(|card| card.course.id).collect();
    let standing: HashMap<Uuid, WorkFacts> = repo::student_work(conn, actor.user_id(), &ids, now)
        .await?
        .into_iter()
        .map(|facts| (facts.assignment_id, facts))
        .collect();
    let items = repo::open_assignments(conn, &ids, now)
        .await?
        .into_iter()
        .filter_map(|row| {
            let facts = standing.get(&row.assignment_id)?;
            Some(WorkEntry {
                assignment_id: row.assignment_id,
                course_id: row.course_id,
                title: row.title,
                assignment_type: row.assignment_type,
                due_at: row.due_at,
                class_on: row.class_on,
                max_points: row.max_points,
                status: facts.status,
                score: facts.score,
                published_at: facts.published_at,
                draft_saved_at: facts.draft_saved_at,
            })
        })
        .collect();
    Ok(MyWork { courses, items })
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
        for item in found {
            let Some(scoped) = scope.iter().find(|s| s.course.course.id == item.course_id) else {
                continue;
            };
            let mut words = Vec::new();
            flat_text(&Value::Object(item.body.0), &mut words);
            hits.push((
                item.title_match,
                Hit {
                    kind: match item.kind {
                        ItemKind::Text => HitKind::Lecture,
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
    mut patch: ItemPatch,
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
            }
            check_points_total(kind, config, patch.max_points.unwrap_or(current.max_points))?;
            // The structure is frozen once work was submitted (ADR-0023); the assignment row is
            // locked, so no submit slips in between this check and the write.
            if patch.config.is_some()
                && kind.structure(&current.config) != kind.structure(config)
                && assessment::has_submitted_for_assignment(conn, id).await?
            {
                return Err(ContentError::StructureFrozen);
            }
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
mod tests;
