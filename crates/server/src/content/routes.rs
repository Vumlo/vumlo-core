//! HTTP edge of `content`: extractors, DTO mapping, OpenAPI annotations. Writes open the
//! transaction, call exactly one service function and commit; reads use a pooled connection.

use api_types::{
    ContentModule, ContentStatus, ContentSubModule, ContentTree, CourseRef, CreateItemRequest,
    CreateModuleRequest, EditorProfiles, FieldError, HitKind, HitStatus, Item, ItemMeta,
    ItemMetaMap, ItemRevision, ItemSummary, Module, MyWork, NewAssignment, ProblemDetails,
    PublishModuleRequest, ReorderItemsRequest, ReorderModulesRequest, SearchHit as ApiHit,
    SearchQuery, SearchResults, UpdateItemRequest, UpdateModuleRequest, WorkCourse, WorkItem,
};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::model::{
    self, AssignmentType, BuiltinType, ContentTitle, ItemKind, ItemPatch, ItemStatus, MaxPoints,
    ModelError, ModulePatch, ModuleStatus, OnDeadline, SettingsPatch, SubmissionSettings,
};
use super::service::{self, ContentError, ItemRules};
use crate::actor::Actor;
use crate::app::AppState;
use crate::error::{ApiError, ValidJson, ValidQuery};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(create_module))
        .routes(routes!(get_tree))
        .routes(routes!(update_module, delete_module))
        .routes(routes!(reorder_modules))
        .routes(routes!(publish_module))
        .routes(routes!(create_item, reorder_items))
        .routes(routes!(get_item, update_item, delete_item))
        .routes(routes!(publish_item))
        .routes(routes!(get_revision))
        .routes(routes!(publish_revision))
        .routes(routes!(get_item_meta))
        .routes(routes!(get_my_work))
        .routes(routes!(search))
        .routes(routes!(get_editor_profiles))
}

impl From<ContentError> for ApiError {
    fn from(error: ContentError) -> Self {
        match error {
            ContentError::Forbidden(policy) => policy.into(),
            ContentError::Invalid(ref model) => {
                let field = match model {
                    ModelError::Title => "title",
                    ModelError::MaxPoints => "max_points",
                    ModelError::AssignmentType => "type",
                    ModelError::Config(_) => "config",
                };
                Self::invalid_field(field, error.to_string())
            }
            ContentError::Document(errors) => Self::Unprocessable {
                detail: "the request has invalid fields".to_owned(),
                errors: errors.into_field_errors("body"),
            },
            ContentError::NotAnAssignment { field } => {
                Self::invalid_field(field, error.to_string())
            }
            ContentError::Settings(problems) => Self::Unprocessable {
                detail: "the request has invalid fields".to_owned(),
                errors: problems
                    .into_iter()
                    .map(|problem| FieldError {
                        field: problem.field.to_owned(),
                        message: problem.message,
                    })
                    .collect(),
            },
            ContentError::PointsLimit { field, message } => Self::invalid_field(field, message),
            ContentError::InvalidParent => Self::invalid_field("parent_id", error.to_string()),
            ContentError::InvalidOrder => Self::invalid_field("ids", error.to_string()),
            ContentError::ModuleNotFound | ContentError::ItemNotFound => Self::NotFound,
            ContentError::Conflict => Self::Conflict("a concurrent change conflicts, retry"),
            ContentError::BelowGivenScores => {
                Self::Conflict("max_points cannot be lowered below a score that was already given")
            }
            ContentError::ItemNotPublished => {
                Self::Conflict("the item is a draft: publish it first")
            }
            ContentError::HasGrades => Self::Conflict("it has grades: clear them before deleting"),
            ContentError::HasSubmissions => {
                Self::Conflict("it has submitted work: it cannot be deleted")
            }
            ContentError::BadQuery => Self::invalid_field("q", "must be 2 to 100 characters"),
            ContentError::StructureFrozen => {
                Self::Conflict("work was submitted: the questions or fields can no longer change")
            }
            ContentError::Gradebook(e) => e.into(),
            ContentError::Assessment(e) => e.into(),
            ContentError::Teaching(e) => e.into(),
            ContentError::Events(e) => e.into(),
            ContentError::Db(e) => Self::from_db("content", &e),
        }
    }
}

fn api_module_status(status: ModuleStatus) -> ContentStatus {
    match status {
        ModuleStatus::Draft => ContentStatus::Draft,
        ModuleStatus::Published => ContentStatus::Published,
    }
}

fn api_item_status(status: ItemStatus) -> ContentStatus {
    match status {
        ItemStatus::Draft => ContentStatus::Draft,
        ItemStatus::Published => ContentStatus::Published,
    }
}

fn api_kind(kind: ItemKind) -> api_types::ItemKind {
    match kind {
        ItemKind::Text => api_types::ItemKind::Text,
        ItemKind::Assignment => api_types::ItemKind::Assignment,
    }
}

fn domain_kind(kind: api_types::ItemKind) -> ItemKind {
    match kind {
        api_types::ItemKind::Text => ItemKind::Text,
        api_types::ItemKind::Assignment => ItemKind::Assignment,
    }
}

fn api_module(module: model::Module) -> Module {
    Module {
        id: module.id,
        course_id: module.course_id,
        parent_id: module.parent_id,
        position: module.position,
        title: module.title,
        status: api_module_status(module.status),
        publish_at: module.publish_at,
        created_at: module.created_at,
        updated_at: module.updated_at,
    }
}

fn api_assignment(assignment: model::Assignment) -> api_types::Assignment {
    api_types::Assignment {
        assignment_type: assignment.assignment_type.as_str().to_owned(),
        config: assignment.config,
        max_points: assignment.max_points.to_string(),
        is_bonus: assignment.is_bonus,
        due_at: assignment.settings.due_at,
        on_deadline: assignment.settings.on_deadline.map(api_on_deadline),
        accept_late: assignment.settings.accept_late,
        late_until: assignment.settings.late_until,
        max_attempts: assignment.settings.max_attempts,
        class_on: assignment.class_on,
    }
}

fn api_on_deadline(on_deadline: OnDeadline) -> api_types::OnDeadline {
    match on_deadline {
        OnDeadline::KeepDraft => api_types::OnDeadline::KeepDraft,
        OnDeadline::AutoSubmit => api_types::OnDeadline::AutoSubmit,
    }
}

fn api_assignment_summary(assignment: model::AssignmentSummary) -> api_types::AssignmentSummary {
    api_types::AssignmentSummary {
        assignment_type: assignment.assignment_type.as_str().to_owned(),
        max_points: assignment.max_points.to_string(),
        is_bonus: assignment.is_bonus,
        due_at: assignment.settings.due_at,
        on_deadline: assignment.settings.on_deadline.map(api_on_deadline),
        accept_late: assignment.settings.accept_late,
        late_until: assignment.settings.late_until,
        max_attempts: assignment.settings.max_attempts,
    }
}

fn api_item(item: model::Item) -> Item {
    Item {
        id: item.id,
        module_id: item.module_id,
        course_id: item.course_id,
        position: item.position,
        kind: api_kind(item.kind),
        title: item.title,
        body: item.body,
        status: api_item_status(item.status),
        publish_at: item.publish_at,
        assignment: item.assignment.map(api_assignment),
        created_at: item.created_at,
        updated_at: item.updated_at,
    }
}

fn api_summary(item: model::ItemSummary) -> ItemSummary {
    ItemSummary {
        id: item.id,
        kind: api_kind(item.kind),
        title: item.title,
        status: api_item_status(item.status),
        publish_at: item.publish_at,
        position: item.position,
        assignment: item.assignment.map(api_assignment_summary),
    }
}

fn api_tree(modules: Vec<model::ContentModule>) -> ContentTree {
    ContentTree {
        modules: modules
            .into_iter()
            .map(|top| ContentModule {
                module: api_module(top.module),
                submodules: top
                    .submodules
                    .into_iter()
                    .map(|sub| ContentSubModule {
                        module: api_module(sub.module),
                        items: sub.items.into_iter().map(api_summary).collect(),
                    })
                    .collect(),
                items: top.items.into_iter().map(api_summary).collect(),
            })
            .collect(),
    }
}

fn title(raw: &str) -> Result<ContentTitle, ContentError> {
    Ok(ContentTitle::parse(raw)?)
}

/// Parses the assignment object of a create request. Every bad field is reported at once, under
/// its path in the request. The messages are fixed texts: they never repeat what was sent.
fn domain_assignment(new: NewAssignment) -> Result<model::Assignment, ApiError> {
    let assignment_type = BuiltinType::parse(&new.assignment_type);
    let max_points = MaxPoints::parse(&new.max_points);
    // With an unknown type the settings cannot be judged; the type is the field to fix first.
    let settings = match &assignment_type {
        Ok(assignment_type) => SubmissionSettings::default().resolve(
            &SettingsPatch {
                due_at: new.due_at.map(Some),
                on_deadline: new.on_deadline.map(Some),
                accept_late: new.accept_late.map(Some),
                late_until: new.late_until.map(Some),
                max_attempts: new.max_attempts.map(Some),
            },
            assignment_type.takes_submissions(),
        ),
        Err(_) => Ok(SubmissionSettings::default()),
    };
    // A class day belongs to in-class work only (ADR-0049).
    let class_on_misplaced =
        matches!((&assignment_type, new.class_on), (Ok(kind), Some(_)) if kind.takes_submissions());
    let settings = if class_on_misplaced {
        let mut problems = settings.err().unwrap_or_default();
        problems.push(model::SettingProblem {
            field: "class_on",
            message: "only an in-class assignment has a class day".to_owned(),
        });
        Err(problems)
    } else {
        settings
    };
    match (assignment_type, max_points, settings) {
        (Ok(assignment_type), Ok(max_points), Ok(settings)) => Ok(model::Assignment {
            assignment_type,
            config: new.config.unwrap_or_default(),
            max_points,
            is_bonus: new.is_bonus,
            settings,
            class_on: new.class_on,
        }),
        (assignment_type, max_points, settings) => {
            let mut errors: Vec<FieldError> = [
                ("assignment.type", assignment_type.err()),
                ("assignment.max_points", max_points.err()),
            ]
            .into_iter()
            .filter_map(|(field, error)| {
                error.map(|error| FieldError {
                    field: field.to_owned(),
                    message: error.to_string(),
                })
            })
            .collect();
            errors.extend(
                settings
                    .err()
                    .into_iter()
                    .flatten()
                    .map(|problem| FieldError {
                        field: format!("assignment.{}", problem.field),
                        message: problem.message,
                    }),
            );
            Err(ApiError::Unprocessable {
                detail: "the request has invalid fields".to_owned(),
                errors,
            })
        }
    }
}

/// A 422 about the assignment part of a create request names its field under `assignment.`.
/// What item writes read from the configuration.
fn rules(state: &AppState) -> ItemRules<'_> {
    ItemRules {
        scale: state.config.institution.grading_scale,
        profiles: &state.editor,
    }
}

fn under_assignment(error: ContentError) -> ApiError {
    // The document is the item's own `body`, not part of the assignment object.
    if matches!(error, ContentError::Document(_)) {
        return error.into();
    }
    match ApiError::from(error) {
        ApiError::Unprocessable { detail, errors } => ApiError::Unprocessable {
            detail,
            errors: errors
                .into_iter()
                .map(|error| FieldError {
                    field: format!("assignment.{}", error.field),
                    message: error.message,
                })
                .collect(),
        },
        other => other,
    }
}

/// Creates a draft module as the last of its siblings. Staff and admins only.
///
/// Without `parent_id` this is a top-level module. With one, a sub-module of that module, which
/// must be a top-level module of this course.
#[utoipa::path(
    post,
    path = "/courses/{id}/modules",
    tag = "content",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    request_body = CreateModuleRequest,
    responses(
        (status = 201, description = "The new draft module", body = Module),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not change this course's content",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "A concurrent change took the position; retry",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid title, or a parent that is not a top-level module of this course",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn create_module(
    State(state): State<AppState>,
    actor: Actor,
    Path(course_id): Path<Uuid>,
    ValidJson(request): ValidJson<CreateModuleRequest>,
) -> Result<(StatusCode, Json<Module>), ApiError> {
    let title = title(&request.title)?;
    let mut tx = state.db.begin().await.map_err(ContentError::from)?;
    let module =
        service::create_module(&mut tx, &actor, course_id, title, request.parent_id).await?;
    tx.commit().await.map_err(ContentError::from)?;
    Ok((StatusCode::CREATED, Json(api_module(module))))
}

/// The whole content tree of a course: top-level modules in order, each with its sub-modules and
/// its direct items, and each sub-module with its items. Items are summaries without the body.
///
/// Staff and admins see everything. A student sees only what is published and not scheduled
/// later, on every level, and nothing at all while the course is a draft. The tree is not paged:
/// it is bounded and ordered by position, not by id (ADR-0018 pages id-ordered lists).
#[utoipa::path(
    get,
    path = "/courses/{id}/content",
    tag = "content",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    responses(
        (status = 200, description = "The content tree as the caller may see it", body = ContentTree),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn get_tree(
    State(state): State<AppState>,
    actor: Actor,
    Path(course_id): Path<Uuid>,
) -> Result<Json<ContentTree>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(ContentError::from)?;
    let modules = service::get_tree(&mut conn, &actor, course_id).await?;
    Ok(Json(api_tree(modules)))
}

/// Changes a module's title and/or `publish_at` (`null` clears it). The parent cannot change.
#[utoipa::path(
    patch,
    path = "/modules/{id}",
    tag = "content",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Module id")),
    request_body = UpdateModuleRequest,
    responses(
        (status = 200, description = "The module", body = Module),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not change this course's content",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such module, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid title or timestamp", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn update_module(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<UpdateModuleRequest>,
) -> Result<Json<Module>, ApiError> {
    let patch = ModulePatch {
        title: request.title.as_deref().map(title).transpose()?,
        publish_at: request.publish_at,
    };
    let mut tx = state.db.begin().await.map_err(ContentError::from)?;
    let module = service::update_module(&mut tx, &actor, id, patch).await?;
    tx.commit().await.map_err(ContentError::from)?;
    Ok(Json(api_module(module)))
}

/// Deletes a module together with its sub-modules and all their items. Drafts of students and
/// empty grade rows go with it. It is refused (409) when an assignment below has a grade or a
/// submitted attempt.
#[utoipa::path(
    delete,
    path = "/modules/{id}",
    tag = "content",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Module id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not change this course's content",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such module, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "An assignment below has a grade or a submitted attempt",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn delete_module(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(ContentError::from)?;
    service::delete_module(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(ContentError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Sets the order of the top-level modules, or of the sub-modules of `parent_id`.
///
/// `ids` must be exactly the current members of that sequence, each once. Positions are
/// rewritten to 1..n in one transaction.
#[utoipa::path(
    put,
    path = "/courses/{id}/modules/order",
    tag = "content",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    request_body = ReorderModulesRequest,
    responses(
        (status = 204, description = "Reordered"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not change this course's content",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "A concurrent change conflicts; retry", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "ids are not exactly the current siblings, or the parent is not a top-level module of this course",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn reorder_modules(
    State(state): State<AppState>,
    actor: Actor,
    Path(course_id): Path<Uuid>,
    ValidJson(request): ValidJson<ReorderModulesRequest>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(ContentError::from)?;
    service::reorder_modules(&mut tx, &actor, course_id, request.parent_id, &request.ids).await?;
    tx.commit().await.map_err(ContentError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Publishes a module. `cascade` is required and has no default (ADR-0010).
///
/// With `cascade: false` only this module is published. With `cascade: true` its sub-modules and
/// every draft item below it are published too, in the same transaction. Idempotent. Never
/// changes `publish_at`: a module scheduled for later stays hidden from students until then.
#[utoipa::path(
    post,
    path = "/modules/{id}/publish",
    tag = "content",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Module id")),
    request_body = PublishModuleRequest,
    responses(
        (status = 200, description = "The module", body = Module),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not change this course's content",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such module, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`cascade` is missing", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn publish_module(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<PublishModuleRequest>,
) -> Result<Json<Module>, ApiError> {
    let mut tx = state.db.begin().await.map_err(ContentError::from)?;
    let module = service::publish_module(&mut tx, &actor, id, request.cascade).await?;
    tx.commit().await.map_err(ContentError::from)?;
    Ok(Json(api_module(module)))
}

/// Creates a draft `text` item or assignment as the last item of a module or sub-module.
///
/// `body` is a JSON object that is stored and returned as is; an array or a string is rejected.
/// For an assignment it holds the instructions.
///
/// `kind: assignment` requires an `assignment` object `{type, config?, max_points, is_bonus?}`
/// (ADR-0021); a `text` item must not carry one. `type` is `text`, `no_submission`, `quiz` or `form`; the
/// `config` of a `quiz` is its questions and must add up to `max_points` (ADR-0050), that of a `form` its
/// fields (ADR-0051), and the others take an empty one (a `text` assignment may narrow its editor).
/// `max_points` is a decimal string such as `"20"` or `"7.5"`; a JSON number is rejected. The
/// grading scale limits the points (ADR-0022): at most 12 per assignment on `twelve`, at most
/// 100 in total over the regular (not bonus) assignments of the course on `hundred`.
#[utoipa::path(
    post,
    path = "/modules/{id}/items",
    tag = "content",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Module id")),
    request_body = CreateItemRequest,
    responses(
        (status = 201, description = "The new draft item", body = Item),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not change this course's content",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such module, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "A concurrent change took the position; retry",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid title, a body that is not a JSON object, a missing or unexpected `assignment` object, an unknown type or config field, invalid points, or points the grading scale does not allow",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn create_item(
    State(state): State<AppState>,
    actor: Actor,
    Path(module_id): Path<Uuid>,
    ValidJson(request): ValidJson<CreateItemRequest>,
) -> Result<(StatusCode, Json<Item>), ApiError> {
    let title = title(&request.title)?;
    let assignment = match (domain_kind(request.kind), request.assignment) {
        (ItemKind::Text, None) => None,
        (ItemKind::Text, Some(_)) => {
            return Err(ApiError::invalid_field(
                "assignment",
                "only an item of kind `assignment` has this field",
            ));
        }
        (ItemKind::Assignment, None) => {
            return Err(ApiError::invalid_field(
                "assignment",
                "an item of kind `assignment` needs an assignment object",
            ));
        }
        (ItemKind::Assignment, Some(new)) => Some(domain_assignment(new)?),
    };
    let mut tx = state.db.begin().await.map_err(ContentError::from)?;
    let item = service::create_item(
        &mut tx,
        &actor,
        rules(&state),
        module_id,
        title,
        request.body,
        assignment,
    )
    .await
    .map_err(under_assignment)?;
    tx.commit().await.map_err(ContentError::from)?;
    Ok((StatusCode::CREATED, Json(api_item(item))))
}

/// Sets the order of the items of a module. `ids` must be exactly the current items, each once.
#[utoipa::path(
    put,
    path = "/modules/{id}/items/order",
    tag = "content",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Module id")),
    request_body = ReorderItemsRequest,
    responses(
        (status = 204, description = "Reordered"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not change this course's content",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such module, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "A concurrent change conflicts; retry", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "ids are not exactly the current items", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn reorder_items(
    State(state): State<AppState>,
    actor: Actor,
    Path(module_id): Path<Uuid>,
    ValidJson(request): ValidJson<ReorderItemsRequest>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(ContentError::from)?;
    service::reorder_items(&mut tx, &actor, module_id, &request.ids).await?;
    tx.commit().await.map_err(ContentError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// One `text` item or assignment with its body (and, for an assignment, its `assignment` part). A
/// student gets 404 unless the course, every module above it and the item are published and not
/// scheduled later.
#[utoipa::path(
    get,
    path = "/items/{id}",
    tag = "content",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Item id")),
    responses(
        (status = 200, description = "The item", body = Item),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such item, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn get_item(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<Item>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(ContentError::from)?;
    let item = service::get_item(&mut conn, &actor, id).await?;
    Ok(Json(api_item(item)))
}

/// Changes an item's title, body and/or `publish_at` (`null` clears it). A `body` replaces the
/// whole body. The title and body of a published item are a working copy: students keep reading the
/// published one until `POST /items/{id}/revision/publish` (ADR-0049). An assignment's `config`,
/// points and deadline settings apply at once.
///
/// For an assignment `config`, `max_points` and `is_bonus` may change too; the grading scale
/// limits apply when `max_points` is raised or a bonus assignment becomes regular. `class_on` is
/// for `no_submission` assignments. They are rejected for a `text` item. `type` and `kind` cannot change: sending them has no effect.
#[utoipa::path(
    patch,
    path = "/items/{id}",
    tag = "content",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Item id")),
    request_body = UpdateItemRequest,
    responses(
        (status = 200, description = "The item", body = Item),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not change this course's content",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such item, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid title or timestamp, a body that is not a JSON object, assignment fields on a `text` item, an unknown config field, invalid points, or points the grading scale does not allow",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn update_item(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<UpdateItemRequest>,
) -> Result<Json<Item>, ApiError> {
    let patch = ItemPatch {
        title: request.title.as_deref().map(title).transpose()?,
        body: request.body,
        publish_at: request.publish_at,
        config: request.config,
        max_points: request
            .max_points
            .as_deref()
            .map(MaxPoints::parse)
            .transpose()
            .map_err(ContentError::from)?,
        is_bonus: request.is_bonus,
        settings: SettingsPatch {
            due_at: request.due_at,
            on_deadline: request.on_deadline,
            accept_late: request.accept_late,
            late_until: request.late_until,
            max_attempts: request.max_attempts,
        },
        class_on: request.class_on,
    };
    let mut tx = state.db.begin().await.map_err(ContentError::from)?;
    let item = service::update_item(&mut tx, &actor, rules(&state), id, patch).await?;
    tx.commit().await.map_err(ContentError::from)?;
    Ok(Json(api_item(item)))
}

/// Deletes a `text` item or an assignment. Drafts of students and empty grade rows go with an
/// assignment. It is refused (409) when the assignment has a grade or a submitted attempt.
#[utoipa::path(
    delete,
    path = "/items/{id}",
    tag = "content",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Item id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not change this course's content",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such item, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The assignment has a grade or a submitted attempt",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn delete_item(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(ContentError::from)?;
    service::delete_item(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(ContentError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Publishes a `text` item or an assignment. Idempotent; never changes `publish_at`.
#[utoipa::path(
    post,
    path = "/items/{id}/publish",
    tag = "content",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Item id")),
    responses(
        (status = 200, description = "The item", body = Item),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not change this course's content",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such item, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn publish_item(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<Item>, ApiError> {
    let mut tx = state.db.begin().await.map_err(ContentError::from)?;
    let item = service::publish_item(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(ContentError::from)?;
    Ok(Json(api_item(item)))
}

fn api_revision(revision: model::Revision) -> ItemRevision {
    ItemRevision {
        has_unpublished_changes: revision.has_unpublished_changes,
        last_published_at: revision.last_published_at,
        size_bytes: revision.size_bytes,
    }
}

/// Whether a published item has changes that students do not see yet. Staff only.
#[utoipa::path(
    get,
    path = "/items/{id}/revision",
    tag = "content",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Item id")),
    responses(
        (status = 200, description = "The revision", body = ItemRevision),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Only staff see revisions", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such item, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn get_revision(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<ItemRevision>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(ContentError::from)?;
    let revision = service::get_revision(&mut conn, &actor, id).await?;
    Ok(Json(api_revision(revision)))
}

/// Gives students the working title and body of a published item (ADR-0049).
///
/// A draft has nothing to publish a revision of: publish the item (`409`). Not in an archived
/// course.
#[utoipa::path(
    post,
    path = "/items/{id}/revision/publish",
    tag = "content",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Item id")),
    responses(
        (status = 200, description = "The revision after publishing", body = ItemRevision),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not change this course's content",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such item, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The item is a draft, or the course is archived",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn publish_revision(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<ItemRevision>, ApiError> {
    let mut tx = state.db.begin().await.map_err(ContentError::from)?;
    let revision = service::publish_revision(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(ContentError::from)?;
    Ok(Json(api_revision(revision)))
}

/// Counts are far below `u32::MAX`; a database that disagreed would still not overflow the field.
fn count(n: i64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn api_work_status(status: model::WorkStatus) -> api_types::WorkStatus {
    match status {
        model::WorkStatus::NotStarted => api_types::WorkStatus::NotStarted,
        model::WorkStatus::Draft => api_types::WorkStatus::Draft,
        model::WorkStatus::Submitted => api_types::WorkStatus::Submitted,
        model::WorkStatus::Graded => api_types::WorkStatus::Graded,
        model::WorkStatus::Missed => api_types::WorkStatus::Missed,
        model::WorkStatus::InClass => api_types::WorkStatus::InClass,
    }
}

fn api_meta(facts: model::ItemFacts) -> ItemMeta {
    let mut meta = ItemMeta {
        read_minutes: facts.read_minutes.map(count),
        class_on: facts.class_on,
        edited: facts.edited,
        ..ItemMeta::default()
    };
    if let Some(work) = facts.work {
        meta.status = Some(api_work_status(work.status));
        meta.score = work.score.map(|score| format!("{score:.2}"));
        meta.draft_saved_at = work.draft_saved_at;
        meta.published_at = work.published_at;
        meta.attempts_used = Some(count(work.attempts_used));
    }
    if let Some(staff) = facts.staff {
        meta.submitted = Some(count(staff.submitted));
        meta.to_grade = Some(count(staff.to_grade));
        meta.draft_grades = Some(count(staff.draft_grades));
        meta.graded_total = Some(count(staff.graded_total));
    }
    meta
}

/// The facts of every item of the course that the tree does not carry, by item id.
///
/// Everyone gets the reading time of a lecture and the class day of in-class work. Staff also get
/// `edited` and the counts of what to grade; a student gets their own standing, with published
/// values only.
#[utoipa::path(
    get,
    path = "/courses/{id}/item-meta",
    tag = "content",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    responses(
        (status = 200, description = "The facts by item id", body = ItemMetaMap),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn get_item_meta(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<ItemMetaMap>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(ContentError::from)?;
    let facts = service::item_meta(&mut conn, &actor, id).await?;
    Ok(Json(ItemMetaMap {
        items: facts
            .into_iter()
            .map(|(id, facts)| (id.to_string(), api_meta(facts)))
            .collect(),
    }))
}

/// Every assignment of the courses the caller studies in, with the caller's own standing on it
/// (ADR-0052). Only what students can open, with published titles, scores and times.
#[utoipa::path(
    get,
    path = "/me/work",
    tag = "content",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The caller's courses and work", body = MyWork),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn get_my_work(
    State(state): State<AppState>,
    actor: Actor,
) -> Result<Json<MyWork>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(ContentError::from)?;
    let work = service::my_work(&mut conn, &actor, chrono::Utc::now()).await?;
    Ok(Json(MyWork {
        courses: work
            .courses
            .into_iter()
            .map(|card| WorkCourse {
                id: card.course.id,
                code: card.code,
                title: card.course.title,
                term_name: card.term_name,
                owner_name: card.owner_name,
                owner_surname: card.owner_surname,
            })
            .collect(),
        items: work
            .items
            .into_iter()
            .map(|entry| WorkItem {
                assignment_id: entry.assignment_id,
                course_id: entry.course_id,
                title: entry.title,
                assignment_type: entry.assignment_type.as_str().to_owned(),
                due_at: entry.due_at,
                class_on: entry.class_on.map(|day| day.to_string()),
                max_points: entry.max_points.to_string(),
                status: api_work_status(entry.status),
                score: entry.score.map(|score| format!("{score:.2}")),
                published_at: entry.published_at,
                draft_saved_at: entry.draft_saved_at,
            })
            .collect(),
    }))
}

/// Courses, lectures and assignments of the caller's courses that contain `q` (2 to 100
/// characters), title matches first, at most 50 (ADR-0052). A student searches the published copy
/// of what students can open; staff also search drafts and unpublished edits.
#[utoipa::path(
    get,
    path = "/search",
    tag = "content",
    security(("cookie" = []), ("bearer" = [])),
    params(SearchQuery),
    responses(
        (status = 200, description = "The hits", body = SearchResults),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "A search text that is not 2 to 100 characters",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn search(
    State(state): State<AppState>,
    actor: Actor,
    ValidQuery(query): ValidQuery<SearchQuery>,
) -> Result<Json<SearchResults>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(ContentError::from)?;
    let hits = service::search(&mut conn, &actor, &query.q, chrono::Utc::now()).await?;
    Ok(Json(SearchResults {
        items: hits
            .into_iter()
            .map(|hit| ApiHit {
                kind: match hit.kind {
                    model::HitKind::Course => HitKind::Course,
                    model::HitKind::Lecture => HitKind::Lecture,
                    model::HitKind::Assignment => HitKind::Assignment,
                },
                id: hit.id,
                title: hit.title,
                course: CourseRef {
                    course_id: hit.course_id,
                    course_code: hit.course_code,
                    course_title: hit.course_title,
                },
                snippet: hit.snippet,
                due_at: hit.due_at,
                status: if hit.draft {
                    HitStatus::Draft
                } else {
                    HitStatus::Published
                },
            })
            .collect(),
        next_cursor: None,
    }))
}

/// The document editor's profiles as the server enforces them: for each profile the features it
/// allows after the institution's configuration narrowed it, whether it is a single block, and
/// the limits; plus the video providers that are switched on. Any signed-in user may read it.
#[utoipa::path(
    get,
    path = "/editor/profiles",
    tag = "editor",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The effective profiles", body = EditorProfiles),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn get_editor_profiles(
    State(state): State<AppState>,
    actor: Actor,
) -> Result<Json<EditorProfiles>, ApiError> {
    let profiles = service::editor_profiles(&actor, &state.editor)?;
    Ok(Json(profiles.to_dto()))
}
