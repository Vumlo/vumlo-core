//! The course settings, the groups and the totals built on them (ADR-0089). Every function that
//! acts for a user authorizes first, takes the connection it runs on and never begins or commits
//! a transaction, like the rest of the service.

use std::collections::HashSet;

use chrono::Utc;
use serde_json::json;
use sqlx::PgConnection;
use sqlx::types::Decimal;
use uuid::Uuid;

use super::{GradebookError, course_access, visible_rows};
use crate::layout::{
    self, Entry, GroupDef, Layout, Settings, Setup, Suggestion, setup as layout_setup,
};
use crate::model::{Column, FieldProblem, Summary};
use crate::repo::{self, GroupRow, Placement, Rules};
use access::Actor;
use access::{self, Action, CourseAccess};
use events::{self, AuditEntry};
use vumlo_config::GradingScale;

const TITLE_MAX_CHARS: usize = 200;
/// The largest number of marks a group may keep ("best K").
const BEST_K_MAX: i32 = 1000;
/// The most assignments one request may place.
pub const MAX_PLACED: usize = 500;
/// The largest target or cap: what the `numeric(6,2)` columns hold.
const POINTS_MAX: i64 = 9999;

/// One assignment's place, as the layout needs it.
pub(super) struct Spot<'a> {
    pub top_module: Uuid,
    pub top_module_title: &'a str,
    pub group_id: Option<Uuid>,
}

fn def_of(key: Uuid, row: Option<&GroupRow>, title: String) -> GroupDef {
    GroupDef {
        key,
        row: row.map(|row| row.id),
        title,
        cap: row.and_then(|row| row.cap),
        best_k: row.and_then(|row| row.best_k).map(|k| k as u32),
        counts: row.is_none_or(|row| row.counts),
        bonus: row.is_some_and(|row| row.bonus),
    }
}

/// The groups of a layout and the group each assignment is counted in. In `course` the groups are
/// the top-level modules that have an assignment; in `custom` they are the teacher's own groups,
/// empty ones included, and an assignment with no group is in none.
pub(super) fn resolve(
    layout: Layout,
    rows: &[GroupRow],
    spots: &[Spot<'_>],
) -> (Vec<GroupDef>, Vec<Option<Uuid>>) {
    match layout {
        Layout::Course => {
            let mut defs: Vec<GroupDef> = Vec::new();
            for spot in spots {
                if !defs.iter().any(|def| def.key == spot.top_module) {
                    let row = rows
                        .iter()
                        .find(|row| row.module_id == Some(spot.top_module));
                    defs.push(def_of(
                        spot.top_module,
                        row,
                        spot.top_module_title.to_owned(),
                    ));
                }
            }
            let keys = spots.iter().map(|spot| Some(spot.top_module)).collect();
            (defs, keys)
        }
        Layout::Custom => {
            let defs: Vec<GroupDef> = rows
                .iter()
                .filter(|row| row.module_id.is_none())
                .map(|row| def_of(row.id, Some(row), row.title.clone().unwrap_or_default()))
                .collect();
            let keys = spots
                .iter()
                .map(|spot| {
                    spot.group_id
                        .filter(|id| defs.iter().any(|def| def.key == *id))
                })
                .collect();
            (defs, keys)
        }
    }
}

/// What the staff gradebook needs besides the marks: the settings, the groups, and the group key
/// of each column (in the order of `columns`).
pub(super) struct StaffLayout {
    pub settings: Settings,
    pub defs: Vec<GroupDef>,
    pub keys: Vec<Option<Uuid>>,
}

/// Resolves the layout of the course and sets `group` on each column.
pub(super) async fn staff_layout(
    conn: &mut PgConnection,
    course_id: Uuid,
    columns: &mut [Column],
) -> Result<StaffLayout, GradebookError> {
    let settings = repo::settings(conn, course_id).await?;
    let rows = repo::groups(conn, course_id).await?;
    let placements = repo::placements(conn, course_id).await?;
    let spots = spots_for(columns, &placements);
    let (defs, keys) = resolve(settings.layout, &rows, &spots);
    for (column, key) in columns.iter_mut().zip(&keys) {
        column.group = *key;
    }
    Ok(StaffLayout {
        settings,
        defs,
        keys,
    })
}

/// The placements in the order of the columns. Both lists come in tree order from one snapshot;
/// an assignment missing from the placements (it was added in between) is treated as loose.
fn spots_for<'a>(columns: &[Column], placements: &'a [Placement]) -> Vec<Spot<'a>> {
    columns
        .iter()
        .map(|column| {
            placements
                .iter()
                .find(|p| p.assignment_id == column.assignment_id)
                .map(|p| Spot {
                    top_module: p.top_module,
                    top_module_title: &p.top_module_title,
                    group_id: p.group_id,
                })
                .unwrap_or(Spot {
                    top_module: Uuid::nil(),
                    top_module_title: "",
                    group_id: None,
                })
        })
        .collect()
}

/// The entries of one student: the marks of the published slot, or of the draft one.
pub(super) fn entries_of(
    columns: &[Column],
    keys: &[Option<Uuid>],
    mark: impl Fn(Uuid) -> Option<Decimal>,
) -> Vec<Entry> {
    columns
        .iter()
        .zip(keys)
        .map(|(column, key)| Entry {
            assignment_id: column.assignment_id,
            max_points: column.max_points,
            is_bonus: column.is_bonus,
            group: *key,
            score: mark(column.assignment_id),
        })
        .collect()
}

/// The setup status of a course from its own numbers.
pub(super) fn status_of(layout: &StaffLayout, columns: &[Column]) -> (Decimal, Setup) {
    let entries = entries_of(columns, &layout.keys, |_| None);
    let status = layout_setup(&layout.settings, &layout.defs, &entries);
    (status.possible, status)
}

// ---------------------------------------------------------------------------------------------
// What a student sees
// ---------------------------------------------------------------------------------------------

/// What the caller's published marks add up to. Only for a current student; `None` on the
/// twelve-point scale, which has no course total.
pub async fn my_summary(
    conn: &mut PgConnection,
    actor: &Actor,
    scale: GradingScale,
    course_id: Uuid,
) -> Result<Option<Summary>, GradebookError> {
    let access = course_access(conn, actor, course_id).await?;
    access::authorize(actor, Action::ViewOwnGrades(access))?;
    if scale == GradingScale::Twelve {
        return Ok(None);
    }
    let now = Utc::now();
    let rows = visible_rows(conn, actor, course_id, access, now).await?;
    let settings = repo::settings(conn, course_id).await?;
    let group_rows = repo::groups(conn, course_id).await?;
    let spots: Vec<Spot<'_>> = rows
        .iter()
        .map(|row| Spot {
            top_module: row.top_module,
            top_module_title: &row.top_module_title,
            group_id: row.group_id,
        })
        .collect();
    let (defs, keys) = resolve(settings.layout, &group_rows, &spots);
    let entries: Vec<Entry> = rows
        .iter()
        .zip(&keys)
        .map(|(row, key)| Entry {
            assignment_id: row.assignment_id,
            max_points: row.max_points,
            is_bonus: row.is_bonus,
            group: *key,
            score: row.score,
        })
        .collect();
    let outcome = layout::compute(&settings, &defs, &entries);
    // Only the groups that have something to show.
    let shown: HashSet<Uuid> = outcome
        .groups
        .iter()
        .filter_map(|group| group.key)
        .collect();
    let groups = defs
        .into_iter()
        .filter(|def| shown.contains(&def.key))
        .collect();
    Ok(Some(Summary {
        settings,
        groups,
        outcome,
    }))
}

// ---------------------------------------------------------------------------------------------
// What staff set up
// ---------------------------------------------------------------------------------------------

/// The settings, the groups, every assignment with its group, the setup status and the proposed
/// groups: what the Structure mode shows. Staff and admins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Structure {
    pub settings: Settings,
    pub groups: Vec<GroupDef>,
    pub columns: Vec<Column>,
    pub possible: Decimal,
    pub setup: Setup,
    pub suggestions: Vec<Suggestion>,
}

pub async fn structure(
    conn: &mut PgConnection,
    actor: &Actor,
    scale: GradingScale,
    course_id: Uuid,
) -> Result<Structure, GradebookError> {
    let access = course_access(conn, actor, course_id).await?;
    access::authorize(actor, Action::ViewGradebook(access))?;
    if scale == GradingScale::Twelve {
        return Err(GradebookError::TwelveScale);
    }
    let mut columns = repo::columns(conn, course_id).await?;
    let layout = staff_layout(conn, course_id, &mut columns).await?;
    let (possible, setup) = status_of(&layout, &columns);
    let suggestions = if layout.settings.layout == Layout::Custom {
        let loose: Vec<(Uuid, String)> = columns
            .iter()
            .filter(|column| column.group.is_none())
            .map(|column| (column.assignment_id, column.title.clone()))
            .collect();
        layout::suggest(&loose)
    } else {
        Vec::new()
    };
    Ok(Structure {
        settings: layout.settings,
        groups: layout.defs,
        columns,
        possible,
        setup,
        suggestions,
    })
}

async fn manage(
    conn: &mut PgConnection,
    actor: &Actor,
    scale: GradingScale,
    course_id: Uuid,
) -> Result<CourseAccess, GradebookError> {
    let access = course_access(conn, actor, course_id).await?;
    access::authorize(actor, Action::ManageGradebook(access))?;
    if scale == GradingScale::Twelve {
        return Err(GradebookError::TwelveScale);
    }
    Ok(access)
}

async fn audit_course(
    conn: &mut PgConnection,
    actor: &Actor,
    action: &'static str,
    course_id: Uuid,
    meta: serde_json::Value,
) -> Result<(), GradebookError> {
    events::audit_admin(
        conn,
        actor,
        AuditEntry {
            action,
            resource_type: "course",
            resource_id: course_id,
            meta,
        },
    )
    .await?;
    Ok(())
}

fn problem(field: &'static str, message: &str) -> GradebookError {
    GradebookError::Invalid(vec![FieldProblem {
        field,
        message: message.to_owned(),
    }])
}

/// Points for a target or a cap: a decimal string with at most two decimals, above zero for a
/// target and a cap, and below what the column holds.
fn parse_points(raw: &str, field: &'static str) -> Result<Decimal, GradebookError> {
    content::parse_decimal(raw)
        .filter(|value| *value <= Decimal::from(POINTS_MAX))
        .ok_or_else(|| {
            problem(
                field,
                "must be a decimal string with at most two decimals, at most 9999",
            )
        })
}

/// A change of the settings. `pass_mark: Some(None)` clears it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsPatch {
    pub target: Option<String>,
    pub pass_mark: Option<Option<String>>,
    pub layout: Option<Layout>,
}

pub async fn update_settings(
    conn: &mut PgConnection,
    actor: &Actor,
    scale: GradingScale,
    course_id: Uuid,
    patch: SettingsPatch,
) -> Result<Settings, GradebookError> {
    manage(conn, actor, scale, course_id).await?;
    let mut settings = repo::settings(conn, course_id).await?;
    let before = settings.clone();
    if let Some(raw) = &patch.target {
        let target = parse_points(raw, "target")?;
        if target.is_zero() {
            return Err(problem("target", "must be more than zero"));
        }
        settings.target = target;
    }
    if let Some(pass_mark) = &patch.pass_mark {
        settings.pass_mark = pass_mark
            .as_deref()
            .map(|raw| parse_points(raw, "pass_mark"))
            .transpose()?;
    }
    if settings
        .pass_mark
        .is_some_and(|mark| mark > settings.target)
    {
        return Err(problem("pass_mark", "must not be more than the target"));
    }
    if let Some(layout) = patch.layout {
        settings.layout = layout;
    }
    if settings != before {
        repo::save_settings(conn, course_id, &settings).await?;
        audit_course(
            conn,
            actor,
            "gradebook.settings",
            course_id,
            json!({"layout": settings.layout.as_str()}),
        )
        .await?;
    }
    Ok(settings)
}

/// What a group may be told: its rules, and its title when it is one of the teacher's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupInput {
    pub title: Option<String>,
    pub cap: Option<String>,
    pub best_k: Option<i32>,
    pub counts: bool,
    pub bonus: bool,
}

fn rules_of(input: &GroupInput) -> Result<Rules, GradebookError> {
    let cap = input
        .cap
        .as_deref()
        .map(|raw| {
            parse_points(raw, "cap").and_then(|cap| {
                if cap.is_zero() {
                    Err(problem("cap", "must be more than zero"))
                } else {
                    Ok(cap)
                }
            })
        })
        .transpose()?;
    if let Some(k) = input.best_k
        && !(1..=BEST_K_MAX).contains(&k)
    {
        return Err(problem("best_k", "must be from 1 to 1000"));
    }
    if input.bonus && !input.counts {
        return Err(problem("bonus", "a practice group cannot be bonus"));
    }
    Ok(Rules {
        cap,
        best_k: input.best_k,
        counts: input.counts,
        bonus: input.bonus,
    })
}

fn title_of(raw: &str) -> Result<String, GradebookError> {
    let title = raw.trim();
    let plain = !title.chars().any(char::is_control);
    if title.is_empty() || title.chars().count() > TITLE_MAX_CHARS || !plain {
        return Err(problem(
            "title",
            "must be 1 to 200 characters of plain text",
        ));
    }
    Ok(title.to_owned())
}

/// What a key means under the course's current layout.
enum Target {
    /// A top-level module of the course (layout `course`).
    Module(Uuid),
    /// A group of the teacher's own (layout `custom`).
    Custom(GroupRow),
}

async fn target_of(
    conn: &mut PgConnection,
    course_id: Uuid,
    key: Uuid,
) -> Result<Target, GradebookError> {
    match repo::settings(conn, course_id).await?.layout {
        Layout::Course => {
            if repo::is_top_module(conn, course_id, key).await? {
                Ok(Target::Module(key))
            } else {
                Err(GradebookError::NoSuchGroup)
            }
        }
        Layout::Custom => repo::find_custom_group(conn, course_id, key)
            .await?
            .map(Target::Custom)
            .ok_or(GradebookError::NoSuchGroup),
    }
}

/// Adds a group of the teacher's own, with the assignments to put in it. Returns its key.
pub async fn create_group(
    conn: &mut PgConnection,
    actor: &Actor,
    scale: GradingScale,
    course_id: Uuid,
    input: GroupInput,
    assignments: Vec<Uuid>,
) -> Result<Uuid, GradebookError> {
    manage(conn, actor, scale, course_id).await?;
    let title = title_of(input.title.as_deref().unwrap_or_default())?;
    let rules = rules_of(&input)?;
    check_assignments(conn, course_id, &assignments).await?;
    let id = Uuid::now_v7();
    let position = repo::next_position(conn, course_id).await?;
    repo::insert_custom_group(conn, id, course_id, &title, position, &rules).await?;
    if !assignments.is_empty() {
        repo::place(conn, &assignments, Some(id)).await?;
    }
    audit_course(
        conn,
        actor,
        "gradebook.group.create",
        course_id,
        json!({"group_id": id, "assignments": assignments.len()}),
    )
    .await?;
    Ok(id)
}

/// Replaces the rules of a group (and its title when it is one of the teacher's own). `key` is the
/// top-level module in the `course` layout and the group in the `custom` one.
pub async fn save_group(
    conn: &mut PgConnection,
    actor: &Actor,
    scale: GradingScale,
    course_id: Uuid,
    key: Uuid,
    input: GroupInput,
) -> Result<(), GradebookError> {
    manage(conn, actor, scale, course_id).await?;
    let rules = rules_of(&input)?;
    match target_of(conn, course_id, key).await? {
        Target::Module(module_id) => {
            if input.title.is_some() {
                return Err(problem(
                    "title",
                    "a module's group is named after the module",
                ));
            }
            repo::save_module_group(conn, Uuid::now_v7(), course_id, module_id, &rules).await?;
        }
        Target::Custom(row) => {
            let title = input.title.as_deref().map(title_of).transpose()?;
            repo::update_custom_group(conn, row.id, title.as_deref(), &rules).await?;
        }
    }
    audit_course(
        conn,
        actor,
        "gradebook.group.update",
        course_id,
        json!({"group_id": key}),
    )
    .await?;
    Ok(())
}

/// Removes a group of the teacher's own (its assignments go back to "Not sorted"), or, in the
/// `course` layout, clears the rules of a module.
pub async fn delete_group(
    conn: &mut PgConnection,
    actor: &Actor,
    scale: GradingScale,
    course_id: Uuid,
    key: Uuid,
) -> Result<(), GradebookError> {
    manage(conn, actor, scale, course_id).await?;
    let id = match target_of(conn, course_id, key).await? {
        Target::Module(module_id) => repo::find_module_group(conn, course_id, module_id)
            .await?
            .map(|row| row.id),
        Target::Custom(row) => Some(row.id),
    };
    if let Some(id) = id {
        repo::delete_group(conn, course_id, id).await?;
        audit_course(
            conn,
            actor,
            "gradebook.group.delete",
            course_id,
            json!({"group_id": key}),
        )
        .await?;
    }
    Ok(())
}

async fn check_assignments(
    conn: &mut PgConnection,
    course_id: Uuid,
    assignments: &[Uuid],
) -> Result<(), GradebookError> {
    let distinct: HashSet<&Uuid> = assignments.iter().collect();
    if assignments.len() > MAX_PLACED || distinct.len() != assignments.len() {
        return Err(problem(
            "assignments",
            "must list each assignment once, at most 500",
        ));
    }
    if assignments.is_empty() {
        return Ok(());
    }
    if repo::count_course_assignments(conn, course_id, assignments).await?
        != assignments.len() as i64
    {
        return Err(problem("assignments", "must be assignments of this course"));
    }
    Ok(())
}

/// Puts assignments in a group of the teacher's own, or back in "Not sorted" (`group: None`).
/// Only in the `custom` layout: in `course` an assignment is where its module is.
pub async fn place(
    conn: &mut PgConnection,
    actor: &Actor,
    scale: GradingScale,
    course_id: Uuid,
    assignments: Vec<Uuid>,
    group: Option<Uuid>,
) -> Result<(), GradebookError> {
    manage(conn, actor, scale, course_id).await?;
    if repo::settings(conn, course_id).await?.layout != Layout::Custom {
        return Err(GradebookError::WrongLayout);
    }
    check_assignments(conn, course_id, &assignments).await?;
    if let Some(group) = group
        && repo::find_custom_group(conn, course_id, group)
            .await?
            .is_none()
    {
        return Err(GradebookError::NoSuchGroup);
    }
    if assignments.is_empty() {
        return Ok(());
    }
    repo::place(conn, &assignments, group).await?;
    audit_course(
        conn,
        actor,
        "gradebook.place",
        course_id,
        json!({"group_id": group, "assignments": assignments.len()}),
    )
    .await?;
    Ok(())
}
