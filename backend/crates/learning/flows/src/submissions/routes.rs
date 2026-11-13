//! HTTP edge of the flows over student work (ADR-0093). Writes open the transaction, call exactly
//! one service function and commit. The handler reads the clock once and hands it to the service
//! (ADR-0035).

use access::ActorSource;
use api_types::{Attempt, ProblemDetails, SaveDraftRequest, SubmissionDraft};
use axum::Json;
use axum::extract::{FromRef, Path, State};
use axum::http::StatusCode;
use chrono::Utc;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::service;
use crate::{LearningError, LearningState};
use access::Actor;
use assessment::{api_attempt, api_draft};
use common::{ApiError, ValidJson};

pub fn router<S>() -> OpenApiRouter<S>
where
    S: ActorSource + Clone + 'static,
    LearningState: FromRef<S>,
{
    OpenApiRouter::new()
        .routes(routes!(save_draft))
        .routes(routes!(submit))
}

/// Saves the draft of the caller for an assignment, replacing the previous one. Students of the
/// course only.
///
/// `content` is a block document of the `student` profile (ADR-0025) as the institution and the
/// assignment's `config.editor.features` narrow it; a rejection names the path, for example
/// `content.blocks[2].props.level`. An empty document is a valid draft. The last write wins and
/// nothing is versioned. After the submission window has closed (ADR-0035) a save is refused.
/// For a `file` assignment the content names files uploaded with `POST /files?for=assignment:{id}`;
/// a file that does not exist, that someone else uploaded or holds, or whose name the assignment
/// does not accept is `422` on `content.files[i]`. Saving a draft emits no event.
#[utoipa::path(
    put,
    path = "/assignments/{id}/submission/draft",
    tag = "assessment",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Assignment id")),
    request_body = SaveDraftRequest,
    responses(
        (status = 200, description = "The saved draft", body = SubmissionDraft),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the assignment but is not a student of the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such assignment, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The assignment takes no submissions, or the window is closed",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "The content is not a valid document", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn save_draft(
    State(state): State<LearningState>,
    actor: Actor,
    Path(assignment_id): Path<Uuid>,
    ValidJson(request): ValidJson<SaveDraftRequest>,
) -> Result<Json<SubmissionDraft>, ApiError> {
    let now = Utc::now();
    let mut tx = state.db.begin().await.map_err(LearningError::from)?;
    let draft = service::save_draft(
        &mut tx,
        &actor,
        assignment_id,
        &request.content,
        &state.editor,
        now,
    )
    .await?;
    tx.commit().await.map_err(LearningError::from)?;
    Ok(Json(api_draft(draft)))
}

/// Submits the caller's draft as their next attempt. Students of the course only.
///
/// The draft is consumed: a second submit without a new draft is `409`. A submit after `due_at`
/// inside the late window is stored with `is_late: true`; after the window, and after
/// `max_attempts` attempts, it is `409`. A document with no text is `422`. Emits one
/// `submission.submitted` event.
#[utoipa::path(
    post,
    path = "/assignments/{id}/submission/submit",
    tag = "assessment",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Assignment id")),
    responses(
        (status = 201, description = "The new attempt", body = Attempt),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the assignment but is not a student of the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such assignment, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "No draft, the window is closed, the attempt limit is reached, or the assignment takes no submissions",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "The draft is empty or no longer a valid document",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn submit(
    State(state): State<LearningState>,
    actor: Actor,
    Path(assignment_id): Path<Uuid>,
) -> Result<(StatusCode, Json<Attempt>), ApiError> {
    let now = Utc::now();
    let mut tx = state.db.begin().await.map_err(LearningError::from)?;
    let attempt = service::submit(&mut tx, &actor, assignment_id, &state.editor, now).await?;
    tx.commit().await.map_err(LearningError::from)?;
    Ok((StatusCode::CREATED, Json(api_attempt(attempt))))
}
