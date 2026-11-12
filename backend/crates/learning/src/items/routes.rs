//! HTTP edge of the flows over items (ADR-0093): reading, changing and deleting an item and a
//! module reach the grades and the attempts, and the result of a quiz reaches the gradebook.
//! Writes open the transaction, call exactly one service function and commit; reads use a pooled
//! connection.

use access::ActorSource;
use api_types::{
    CreateItemRequest, Item, ProblemDetails, QuizResult, QuizResultQuestion, UpdateItemRequest,
};
use axum::Json;
use axum::extract::{FromRef, Path, State};
use axum::http::StatusCode;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::service;
use crate::LearningError;
use crate::LearningState;
use crate::content::{api_item, item_patch, new_item, under_assignment};
use access::Actor;
use common::{ApiError, ValidJson};

pub fn router<S>() -> OpenApiRouter<S>
where
    S: ActorSource + Clone + 'static,
    LearningState: FromRef<S>,
{
    OpenApiRouter::new()
        .routes(routes!(delete_module))
        .routes(routes!(create_item))
        .routes(routes!(get_item, update_item, delete_item))
        .routes(routes!(get_result))
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
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(LearningError::from)?;
    service::delete_module(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(LearningError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Creates a draft `text` item, assignment or `file` item as the last item of a module or sub-module.
///
/// `body` is a JSON object that is stored and returned as is; an array or a string is rejected.
/// For an assignment it holds the instructions. `kind` is optional: without it the item is an
/// assignment when an `assignment` object is sent and a `text` item otherwise.
///
/// A `file` item has the body `{file_id?, description?}` (`description` is a document). It starts
/// without a file: upload one with `POST /files?for=item:{id}` and name it with
/// `PATCH /items/{id}`. `file_id` must be a file the caller uploaded that nothing uses yet. A
/// `file` item must not carry an `assignment` object.
///
/// `kind: assignment` requires an `assignment` object `{type, config?, max_points, is_bonus?}`
/// (ADR-0021); a `text` item must not carry one. `type` is `text`, `no_submission`, `quiz`, `form` or `file`; the
/// `config` of a `quiz` is its questions and must add up to `max_points` (ADR-0050), that of a `form` its
/// fields (ADR-0051), that of a `file` `{max_files?, accept?}` (ADR-0069), and the others take an empty one (a `text` assignment may narrow its editor).
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
        (status = 422, description = "Invalid title, a body that is not a JSON object (or, for a `file` item, not `{file_id?, description?}` with a usable `file_id`), a missing or unexpected `assignment` object, an unknown type or config field, invalid points, or points the grading scale does not allow",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn create_item(
    State(state): State<LearningState>,
    actor: Actor,
    Path(module_id): Path<Uuid>,
    ValidJson(request): ValidJson<CreateItemRequest>,
) -> Result<(StatusCode, Json<Item>), ApiError> {
    let input = new_item(request)?;
    let mut tx = state.db.begin().await.map_err(LearningError::from)?;
    let item = service::create_item(&mut tx, &actor, state.item_rules(), module_id, input)
        .await
        .map_err(|error| match error {
            LearningError::Content(error) => under_assignment(error),
            other => other.into(),
        })?;
    tx.commit().await.map_err(LearningError::from)?;
    Ok((StatusCode::CREATED, Json(api_item(item))))
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
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<Item>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(LearningError::from)?;
    let item = service::get(&mut conn, &actor, id).await?;
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
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<UpdateItemRequest>,
) -> Result<Json<Item>, ApiError> {
    let patch = item_patch(request)?;
    let mut tx = state.db.begin().await.map_err(LearningError::from)?;
    let item = service::update_item(&mut tx, &actor, state.item_rules(), id, patch).await?;
    tx.commit().await.map_err(LearningError::from)?;
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
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(LearningError::from)?;
    service::delete_item(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(LearningError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The caller's result of a quiz, once their grade is published (ADR-0050 point 8): the published
/// score and feedback, what the server scored for each question, the caller's answers and, when
/// the quiz shows its answers, the correct ones. Students of the course only.
#[utoipa::path(
    get,
    path = "/assignments/{id}/result",
    tag = "assessment",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Assignment id")),
    responses(
        (status = 200, description = "The result", body = QuizResult),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the assignment but is not a student of the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such quiz, not visible to the caller, or no grade published yet",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn get_result(
    State(state): State<LearningState>,
    actor: Actor,
    Path(assignment_id): Path<Uuid>,
) -> Result<Json<QuizResult>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(LearningError::from)?;
    let result = service::result(&mut conn, &actor, assignment_id).await?;
    let points = |value: sqlx::types::Decimal| format!("{value:.2}");
    Ok(Json(QuizResult {
        score: points(result.score),
        max_points: points(result.max_points),
        published_at: result.published_at,
        feedback: result.feedback,
        attempt: result.attempt,
        questions: result
            .questions
            .into_iter()
            .map(|question| QuizResultQuestion {
                id: question.id,
                points: points(question.points),
                got: question.got.map(points),
                answer: question.answer,
                correct: question.correct,
                explanation: question.explanation,
            })
            .collect(),
    }))
}
