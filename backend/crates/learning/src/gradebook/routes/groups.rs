//! HTTP edge of the course settings, the groups and the totals (ADR-0089).

use api_types::{
    CreatedGradebookGroup, GradebookGroup, GradebookGroupInput, GradebookGroupResult,
    GradebookLayout, GradebookLayoutView, GradebookOutcome, GradebookPlacement, GradebookSettings,
    GradebookSetup, GradebookStructure, GradebookSuggestion, GradebookTotals, MyGradeSummary,
    ProblemDetails, SummaryGroup,
};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::{api_column, points};
use crate::LearningState;
use crate::gradebook::layout::{GroupDef, Layout, Outcome, Settings, Setup};
use crate::gradebook::model::{LayoutView, RowTotals, Summary};
use crate::gradebook::service::{self, GradebookError, GroupInput, SettingsPatch};
use access::Actor;
use common::{ApiError, ValidJson};

pub fn router<S>() -> OpenApiRouter<S>
where
    S: access::ActorSource + Clone + 'static,
    LearningState: axum::extract::FromRef<S>,
{
    OpenApiRouter::new()
        .routes(routes!(get_structure))
        .routes(routes!(put_settings))
        .routes(routes!(post_group))
        .routes(routes!(put_group, delete_group))
        .routes(routes!(put_placement))
        .routes(routes!(get_my_summary))
}

fn api_layout(layout: Layout) -> GradebookLayout {
    match layout {
        Layout::Course => GradebookLayout::Course,
        Layout::Custom => GradebookLayout::Custom,
    }
}

fn model_layout(layout: GradebookLayout) -> Layout {
    match layout {
        GradebookLayout::Course => Layout::Course,
        GradebookLayout::Custom => Layout::Custom,
    }
}

pub(super) fn api_settings(settings: &Settings) -> GradebookSettings {
    GradebookSettings {
        target: points(settings.target),
        pass_mark: settings.pass_mark.map(points),
        layout: api_layout(settings.layout),
    }
}

fn api_group(def: &GroupDef) -> GradebookGroup {
    GradebookGroup {
        key: def.key,
        title: def.title.clone(),
        cap: def.cap.map(points),
        best_k: def.best_k,
        counts: def.counts,
        bonus: def.bonus,
    }
}

fn api_setup(setup: &Setup) -> GradebookSetup {
    GradebookSetup {
        unsorted: setup.unsorted,
        empty_groups: setup.empty_groups.clone(),
        possible: points(setup.possible),
        target: points(setup.target),
        reaches_target: setup.reaches_target,
    }
}

pub(super) fn api_layout_view(view: LayoutView) -> GradebookLayoutView {
    GradebookLayoutView {
        settings: api_settings(&view.settings),
        groups: view.groups.iter().map(api_group).collect(),
        possible: points(view.possible),
        setup: api_setup(&view.setup),
    }
}

fn api_outcome(outcome: Outcome) -> GradebookOutcome {
    GradebookOutcome {
        total: points(outcome.total),
        earned: points(outcome.raw_total),
        passed: outcome.passed,
        groups: outcome
            .groups
            .into_iter()
            .map(|group| GradebookGroupResult {
                key: group.key,
                total: points(group.total),
                capped: group.capped,
                dropped: group.dropped,
            })
            .collect(),
    }
}

pub(super) fn api_totals(totals: RowTotals) -> GradebookTotals {
    GradebookTotals {
        published: api_outcome(totals.published),
        draft: api_outcome(totals.draft),
    }
}

fn api_summary(summary: Summary) -> MyGradeSummary {
    let Summary {
        settings,
        groups,
        outcome,
    } = summary;
    let mut shown = Vec::new();
    let mut loose = Vec::new();
    let mut loose_total = points(Default::default());
    for result in outcome.groups {
        match result.key {
            Some(key) => {
                if let Some(def) = groups.iter().find(|def| def.key == key) {
                    shown.push(SummaryGroup {
                        group: api_group(def),
                        total: points(result.total),
                        possible: points(result.possible),
                        capped: result.capped,
                        assignments: result.assignments,
                        dropped: result.dropped,
                    });
                }
            }
            None => {
                loose_total = points(result.total);
                loose = result.assignments;
            }
        }
    }
    MyGradeSummary {
        settings: api_settings(&settings),
        total: points(outcome.total),
        earned: points(outcome.raw_total),
        possible: points(outcome.possible),
        passed: outcome.passed,
        groups: shown,
        loose,
        loose_total,
    }
}

fn group_input(input: GradebookGroupInput) -> (GroupInput, Vec<Uuid>) {
    (
        GroupInput {
            title: input.title,
            cap: input.cap,
            best_k: input.best_k,
            counts: input.counts,
            bonus: input.bonus,
        },
        input.assignments,
    )
}

/// The settings, the groups, every assignment with its group, the setup status and the proposed
/// groups: what the Structure mode of the gradebook shows. Staff and admins.
///
/// There is no structure on the twelve-point scale: the answer is `409`.
#[utoipa::path(
    get,
    path = "/courses/{id}/gradebook/structure",
    tag = "gradebook",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    responses(
        (status = 200, description = "The structure", body = GradebookStructure),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the course but not its gradebook",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The twelve-point scale has no course total",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn get_structure(
    State(state): State<LearningState>,
    actor: Actor,
    Path(course_id): Path<Uuid>,
) -> Result<Json<GradebookStructure>, ApiError> {
    let scale = state.config.institution.grading_scale;
    let mut conn = state.db.acquire().await.map_err(GradebookError::from)?;
    let structure = service::structure(&mut conn, &actor, scale, course_id).await?;
    Ok(Json(GradebookStructure {
        settings: api_settings(&structure.settings),
        groups: structure.groups.iter().map(api_group).collect(),
        columns: structure.columns.into_iter().map(api_column).collect(),
        possible: points(structure.possible),
        setup: api_setup(&structure.setup),
        suggestions: structure
            .suggestions
            .into_iter()
            .map(|suggestion| GradebookSuggestion {
                title: suggestion.title,
                assignments: suggestion.assignments,
            })
            .collect(),
    }))
}

/// Replaces the target, the pass mark and the layout of a course. Staff and admins; not in an
/// archived course; `409` on the twelve-point scale.
///
/// The layout can be switched back and forth: nothing of the other layout is deleted.
#[utoipa::path(
    put,
    path = "/courses/{id}/gradebook/settings",
    tag = "gradebook",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    request_body = GradebookSettings,
    responses(
        (status = 200, description = "The settings", body = GradebookSettings),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the course but not change it",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The twelve-point scale has no course total",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid `target` or `pass_mark`", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn put_settings(
    State(state): State<LearningState>,
    actor: Actor,
    Path(course_id): Path<Uuid>,
    ValidJson(request): ValidJson<GradebookSettings>,
) -> Result<Json<GradebookSettings>, ApiError> {
    let scale = state.config.institution.grading_scale;
    let patch = SettingsPatch {
        target: Some(request.target),
        pass_mark: Some(request.pass_mark),
        layout: Some(model_layout(request.layout)),
    };
    let mut tx = state.db.begin().await.map_err(GradebookError::from)?;
    let settings = service::update_settings(&mut tx, &actor, scale, course_id, patch).await?;
    tx.commit().await.map_err(GradebookError::from)?;
    Ok(Json(api_settings(&settings)))
}

/// Adds a group of the teacher's own, with the assignments to put in it. Staff and admins.
#[utoipa::path(
    post,
    path = "/courses/{id}/gradebook/groups",
    tag = "gradebook",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    request_body = GradebookGroupInput,
    responses(
        (status = 201, description = "The key of the new group", body = CreatedGradebookGroup),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the course but not change it",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The twelve-point scale has no course total",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid title, rules or assignments", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn post_group(
    State(state): State<LearningState>,
    actor: Actor,
    Path(course_id): Path<Uuid>,
    ValidJson(request): ValidJson<GradebookGroupInput>,
) -> Result<(StatusCode, Json<CreatedGradebookGroup>), ApiError> {
    let scale = state.config.institution.grading_scale;
    let (input, assignments) = group_input(request);
    let mut tx = state.db.begin().await.map_err(GradebookError::from)?;
    let key = service::create_group(&mut tx, &actor, scale, course_id, input, assignments).await?;
    tx.commit().await.map_err(GradebookError::from)?;
    Ok((StatusCode::CREATED, Json(CreatedGradebookGroup { key })))
}

/// Replaces the rules of a group. Staff and admins.
///
/// `key` is the top-level module in the `course` layout (its title is the module's, so a `title`
/// is refused) and the group in the `custom` one.
#[utoipa::path(
    put,
    path = "/courses/{id}/gradebook/groups/{key}",
    tag = "gradebook",
    security(("cookie" = []), ("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Course id"),
        ("key" = Uuid, Path, description = "A module (layout `course`) or a group (layout `custom`)"),
    ),
    request_body = GradebookGroupInput,
    responses(
        (status = 204, description = "Saved"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the course but not change it",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course or group", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 409, description = "The twelve-point scale has no course total",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid title or rules", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn put_group(
    State(state): State<LearningState>,
    actor: Actor,
    Path((course_id, key)): Path<(Uuid, Uuid)>,
    ValidJson(request): ValidJson<GradebookGroupInput>,
) -> Result<StatusCode, ApiError> {
    let scale = state.config.institution.grading_scale;
    let (input, _) = group_input(request);
    let mut tx = state.db.begin().await.map_err(GradebookError::from)?;
    service::save_group(&mut tx, &actor, scale, course_id, key, input).await?;
    tx.commit().await.map_err(GradebookError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Removes a group of the teacher's own (its assignments go back to "Not sorted"), or, in the
/// `course` layout, clears the rules of a module. Staff and admins.
#[utoipa::path(
    delete,
    path = "/courses/{id}/gradebook/groups/{key}",
    tag = "gradebook",
    security(("cookie" = []), ("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Course id"),
        ("key" = Uuid, Path, description = "A module (layout `course`) or a group (layout `custom`)"),
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the course but not change it",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course or group", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 409, description = "The twelve-point scale has no course total",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn delete_group(
    State(state): State<LearningState>,
    actor: Actor,
    Path((course_id, key)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let scale = state.config.institution.grading_scale;
    let mut tx = state.db.begin().await.map_err(GradebookError::from)?;
    service::delete_group(&mut tx, &actor, scale, course_id, key).await?;
    tx.commit().await.map_err(GradebookError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Puts assignments in a group of the teacher's own, or back in "Not sorted". Staff and admins.
/// Only in the `custom` layout: in `course` an assignment is where its module is (`409`).
#[utoipa::path(
    put,
    path = "/courses/{id}/gradebook/placement",
    tag = "gradebook",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    request_body = GradebookPlacement,
    responses(
        (status = 204, description = "Placed"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the course but not change it",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course or group", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 409, description = "The course is not in the custom layout, or is on the \
            twelve-point scale", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Not assignments of this course", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn put_placement(
    State(state): State<LearningState>,
    actor: Actor,
    Path(course_id): Path<Uuid>,
    ValidJson(request): ValidJson<GradebookPlacement>,
) -> Result<StatusCode, ApiError> {
    let scale = state.config.institution.grading_scale;
    let mut tx = state.db.begin().await.map_err(GradebookError::from)?;
    service::place(
        &mut tx,
        &actor,
        scale,
        course_id,
        request.assignments,
        request.group,
    )
    .await?;
    tx.commit().await.map_err(GradebookError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// What the caller's published marks add up to in a course: the total against the target, each
/// group with its rules, and which marks count. Students only; built from published values and
/// from assignments the caller may see now, never from a draft.
#[utoipa::path(
    get,
    path = "/courses/{id}/my-grade-summary",
    tag = "gradebook",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    responses(
        (status = 200, description = "The summary", body = MyGradeSummary),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller is not a student of the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The twelve-point scale has no course total",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn get_my_summary(
    State(state): State<LearningState>,
    actor: Actor,
    Path(course_id): Path<Uuid>,
) -> Result<Json<MyGradeSummary>, ApiError> {
    let scale = state.config.institution.grading_scale;
    let mut conn = state.db.acquire().await.map_err(GradebookError::from)?;
    let summary = service::my_summary(&mut conn, &actor, scale, course_id)
        .await?
        .ok_or(GradebookError::TwelveScale)?;
    Ok(Json(api_summary(summary)))
}
