//! HTTP edge of `content`: extractors, DTO mapping, OpenAPI annotations. Writes open the
//! transaction, call exactly one service function and commit; reads use a pooled connection.

use api_types::{
    ContentModule, ContentStatus, ContentSubModule, ContentTree, CreateItemRequest,
    CreateModuleRequest, Item, ItemSummary, Module, ProblemDetails, PublishModuleRequest,
    ReorderItemsRequest, ReorderModulesRequest, UpdateItemRequest, UpdateModuleRequest,
};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::model::{
    self, ContentTitle, ItemKind, ItemPatch, ItemStatus, ModelError, ModulePatch, ModuleStatus,
};
use super::service::{self, ContentError};
use crate::actor::Actor;
use crate::app::AppState;
use crate::error::{ApiError, ValidJson};

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
}

impl From<ContentError> for ApiError {
    fn from(error: ContentError) -> Self {
        match error {
            ContentError::Forbidden(_) => Self::Forbidden("not allowed"),
            ContentError::Invalid(ModelError::Title) => {
                Self::invalid_field("title", error.to_string())
            }
            ContentError::InvalidParent => Self::invalid_field("parent_id", error.to_string()),
            ContentError::InvalidOrder => Self::invalid_field("ids", error.to_string()),
            ContentError::ModuleNotFound | ContentError::ItemNotFound => Self::NotFound,
            ContentError::Conflict => Self::Conflict("a concurrent change conflicts, retry"),
            ContentError::Teaching(e) => e.into(),
            ContentError::Events(e) => {
                tracing::error!(error = %e, "content: audit log error");
                Self::Internal
            }
            ContentError::Db(e) => {
                tracing::error!(error = %e, "content: database error");
                Self::Internal
            }
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
        ItemKind::Lesson => api_types::ItemKind::Lesson,
    }
}

/// Assignments get their service support with the next change; until then they are refused.
fn domain_kind(kind: api_types::ItemKind) -> Result<ItemKind, ApiError> {
    match kind {
        api_types::ItemKind::Lesson => Ok(ItemKind::Lesson),
        api_types::ItemKind::Assignment => Err(ApiError::invalid_field(
            "kind",
            "assignments cannot be created yet",
        )),
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
        assignment: None,
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
        assignment: None,
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

/// Deletes a module together with its sub-modules and all their items.
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

/// Creates a draft lesson as the last item of a module or sub-module.
///
/// `body` is a JSON object that is stored and returned as is; an array or a string is rejected.
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
        (status = 422, description = "Invalid title, or a body that is not a JSON object",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn create_item(
    State(state): State<AppState>,
    actor: Actor,
    Path(module_id): Path<Uuid>,
    ValidJson(request): ValidJson<CreateItemRequest>,
) -> Result<(StatusCode, Json<Item>), ApiError> {
    let kind = domain_kind(request.kind)?;
    if request.assignment.is_some() {
        return Err(ApiError::invalid_field(
            "assignment",
            "only an item of kind `assignment` has this field",
        ));
    }
    let title = title(&request.title)?;
    let mut tx = state.db.begin().await.map_err(ContentError::from)?;
    let item = service::create_item(&mut tx, &actor, module_id, kind, title, request.body).await?;
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

/// One lesson with its body. A student gets 404 unless the course, every module above it and the
/// lesson are published and not scheduled later.
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

/// Changes a lesson's title, body and/or `publish_at` (`null` clears it). A `body` replaces the
/// whole body. Edits to a published lesson are visible at once (ADR-0010).
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
        (status = 422, description = "Invalid title or timestamp, or a body that is not a JSON object",
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
    };
    let mut tx = state.db.begin().await.map_err(ContentError::from)?;
    let item = service::update_item(&mut tx, &actor, id, patch).await?;
    tx.commit().await.map_err(ContentError::from)?;
    Ok(Json(api_item(item)))
}

/// Deletes a lesson.
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

/// Publishes a lesson. Idempotent; never changes `publish_at`.
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
