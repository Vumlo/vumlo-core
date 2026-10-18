//! HTTP edge of `assessment`: extractors, DTO mapping, OpenAPI annotations. Writes open the
//! transaction, call exactly one service function and commit; reads use a pooled connection. The
//! handler reads the clock once and hands it to the service (ADR-0035).

use api_types::{
    Attempt, AttemptsPage, FormResponseRow, FormResponses, LatestAttempt, MySubmission, PageQuery,
    ProblemDetails, QuizResult, QuizResultQuestion, SaveDraftRequest, StudentName, SubmissionDraft,
    SubmissionRow, SubmissionStatus, SubmissionsPage,
};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::Utc;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::model;
use super::service::{self, AssessmentError};
use crate::actor::Actor;
use crate::app::AppState;
use crate::error::{ApiError, ValidJson, ValidQuery};
use crate::pagination::PageParams;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(save_draft, discard_draft))
        .routes(routes!(get_my_submission))
        .routes(routes!(submit))
        .routes(routes!(list_submissions))
        .routes(routes!(list_student_attempts))
        .routes(routes!(get_result))
        .routes(routes!(get_responses))
}

impl From<AssessmentError> for ApiError {
    fn from(error: AssessmentError) -> Self {
        match error {
            AssessmentError::Forbidden(policy) => policy.into(),
            AssessmentError::AssignmentNotFound | AssessmentError::StudentNotFound => {
                Self::NotFound
            }
            AssessmentError::TakesNoSubmissions => {
                Self::Conflict("this assignment takes no submissions")
            }
            AssessmentError::WindowClosed => Self::Conflict("the submission window is closed"),
            AssessmentError::NoDraft => Self::Conflict("there is no draft to submit"),
            AssessmentError::AttemptLimit => Self::Conflict("the attempt limit is reached"),
            AssessmentError::Concurrent => {
                Self::Conflict("a concurrent submit took this attempt, retry")
            }
            AssessmentError::HasSubmissions => Self::Conflict("it has submitted work"),
            AssessmentError::OptionTaken => {
                Self::Conflict("an exclusive option was taken by someone else")
            }
            AssessmentError::NothingToShow => Self::NotFound,
            AssessmentError::Gradebook(e) => e.into(),
            AssessmentError::Document(errors) => Self::Unprocessable {
                detail: "the request has invalid fields".to_owned(),
                errors: errors.into_field_errors("content"),
            },
            AssessmentError::Empty => {
                Self::invalid_field("content", "must have some text before it is submitted")
            }
            AssessmentError::Teaching(e) => e.into(),
            AssessmentError::Events(e) => e.into(),
            AssessmentError::Db(e) => Self::from_db("assessment", &e),
        }
    }
}

fn page_params(query: &PageQuery) -> Result<PageParams, ApiError> {
    Ok(PageParams::new(query.cursor.as_deref(), query.limit)?)
}

fn api_draft(draft: model::Draft) -> SubmissionDraft {
    SubmissionDraft {
        content: draft.content,
        updated_at: draft.updated_at,
    }
}

fn api_attempt(attempt: model::Attempt) -> Attempt {
    Attempt {
        id: attempt.id,
        attempt: attempt.attempt,
        content: attempt.content,
        submitted_at: attempt.submitted_at,
        is_late: attempt.is_late,
        auto_scores: attempt.auto_scores,
    }
}

fn api_name(student: model::StudentName) -> StudentName {
    StudentName {
        user_id: student.user_id,
        name: student.name,
        surname: student.surname,
    }
}

fn api_row(row: model::StudentSubmission) -> SubmissionRow {
    SubmissionRow {
        student_id: row.student_id,
        name: row.name,
        surname: row.surname,
        former: row.former,
        status: if row.latest.is_some() {
            SubmissionStatus::Submitted
        } else {
            SubmissionStatus::NotSubmitted
        },
        latest: row.latest.map(|latest| LatestAttempt {
            id: latest.id,
            attempt: latest.attempt,
            submitted_at: latest.submitted_at,
            is_late: latest.is_late,
        }),
    }
}

/// Saves the draft of the caller for an assignment, replacing the previous one. Students of the
/// course only.
///
/// `content` is a block document of the `student` profile (ADR-0025) as the institution and the
/// assignment's `config.editor.features` narrow it; a rejection names the path, for example
/// `content.blocks[2].props.level`. An empty document is a valid draft. The last write wins and
/// nothing is versioned. After the submission window has closed (ADR-0035) a save is refused.
/// Saving a draft emits no event.
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
    State(state): State<AppState>,
    actor: Actor,
    Path(assignment_id): Path<Uuid>,
    ValidJson(request): ValidJson<SaveDraftRequest>,
) -> Result<Json<SubmissionDraft>, ApiError> {
    let now = Utc::now();
    let mut tx = state.db.begin().await.map_err(AssessmentError::from)?;
    let draft = service::save_draft(
        &mut tx,
        &actor,
        assignment_id,
        &request.content,
        &state.editor,
        now,
    )
    .await?;
    tx.commit().await.map_err(AssessmentError::from)?;
    Ok(Json(api_draft(draft)))
}

/// Deletes the caller's draft. Succeeds also when there is none, and also after the window has
/// closed. Emits no event.
#[utoipa::path(
    delete,
    path = "/assignments/{id}/submission/draft",
    tag = "assessment",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Assignment id")),
    responses(
        (status = 204, description = "The draft is gone"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the assignment but is not a student of the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such assignment, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The assignment takes no submissions", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn discard_draft(
    State(state): State<AppState>,
    actor: Actor,
    Path(assignment_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(AssessmentError::from)?;
    service::discard_draft(&mut tx, &actor, assignment_id).await?;
    tx.commit().await.map_err(AssessmentError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The caller's own work on an assignment: the draft, if there is one, and the submitted attempts,
/// newest first and paged. A client prefills a new draft from `attempts[0]`. Students of the
/// course only; nobody else's work is reachable here.
#[utoipa::path(
    get,
    path = "/assignments/{id}/submission",
    tag = "assessment",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Assignment id"), PageQuery),
    responses(
        (status = 200, description = "The draft and a page of attempts", body = MySubmission),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the assignment but is not a student of the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such assignment, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The assignment takes no submissions", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn get_my_submission(
    State(state): State<AppState>,
    actor: Actor,
    Path(assignment_id): Path<Uuid>,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<MySubmission>, ApiError> {
    let page = page_params(&query)?;
    let mut conn = state.db.acquire().await.map_err(AssessmentError::from)?;
    let mine = service::my_submission(&mut conn, &actor, assignment_id, page).await?;
    Ok(Json(MySubmission {
        draft: mine.draft.map(api_draft),
        attempts: mine.attempts.items.into_iter().map(api_attempt).collect(),
        next_cursor: mine.attempts.next.map(|id| id.to_string()),
    }))
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
    State(state): State<AppState>,
    actor: Actor,
    Path(assignment_id): Path<Uuid>,
) -> Result<(StatusCode, Json<Attempt>), ApiError> {
    let now = Utc::now();
    let mut tx = state.db.begin().await.map_err(AssessmentError::from)?;
    let attempt = service::submit(&mut tx, &actor, assignment_id, &state.editor, now).await?;
    tx.commit().await.map_err(AssessmentError::from)?;
    Ok((StatusCode::CREATED, Json(api_attempt(attempt))))
}

/// The current students of the course with their latest submitted attempt, without its content, by
/// surname, name and id. A student who has not submitted is listed with `status: not_submitted`.
/// A draft is never shown. Staff and admins.
#[utoipa::path(
    get,
    path = "/assignments/{id}/submissions",
    tag = "assessment",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Assignment id"), PageQuery),
    responses(
        (status = 200, description = "A page of students", body = SubmissionsPage),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the assignment but not its submissions",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such assignment, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The assignment takes no submissions", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn list_submissions(
    State(state): State<AppState>,
    actor: Actor,
    Path(assignment_id): Path<Uuid>,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<SubmissionsPage>, ApiError> {
    let page = page_params(&query)?;
    let mut conn = state.db.acquire().await.map_err(AssessmentError::from)?;
    let students = service::list_submissions(&mut conn, &actor, assignment_id, page).await?;
    Ok(Json(SubmissionsPage {
        items: students.items.into_iter().map(api_row).collect(),
        next_cursor: students.next.map(|id| id.to_string()),
    }))
}

/// Every submitted attempt of one student, newest first and paged, with its content. A draft is
/// never shown. Staff and admins; the student is a current student of the course or has an
/// attempt.
#[utoipa::path(
    get,
    path = "/assignments/{id}/submissions/{student_id}",
    tag = "assessment",
    security(("cookie" = []), ("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Assignment id"),
        ("student_id" = Uuid, Path, description = "A student of the course, or one with an attempt"),
        PageQuery,
    ),
    responses(
        (status = 200, description = "A page of attempts", body = AttemptsPage),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the assignment but not its submissions",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such assignment or student, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The assignment takes no submissions", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn list_student_attempts(
    State(state): State<AppState>,
    actor: Actor,
    Path((assignment_id, student_id)): Path<(Uuid, Uuid)>,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<AttemptsPage>, ApiError> {
    let page = page_params(&query)?;
    let mut conn = state.db.acquire().await.map_err(AssessmentError::from)?;
    let attempts =
        service::student_attempts(&mut conn, &actor, assignment_id, student_id, page).await?;
    Ok(Json(AttemptsPage {
        items: attempts.items.into_iter().map(api_attempt).collect(),
        next_cursor: attempts.next.map(|id| id.to_string()),
    }))
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
    State(state): State<AppState>,
    actor: Actor,
    Path(assignment_id): Path<Uuid>,
) -> Result<Json<QuizResult>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(AssessmentError::from)?;
    let result = service::quiz_result(&mut conn, &actor, assignment_id).await?;
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

/// The responses to a form: the latest attempt of every current student who submitted, with their
/// answers, and the students who did not (ADR-0051 point 7). Staff and admins.
#[utoipa::path(
    get,
    path = "/assignments/{id}/responses",
    tag = "assessment",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Assignment id")),
    responses(
        (status = 200, description = "The responses", body = FormResponses),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the assignment but not its submissions",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such form, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn get_responses(
    State(state): State<AppState>,
    actor: Actor,
    Path(assignment_id): Path<Uuid>,
) -> Result<Json<FormResponses>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(AssessmentError::from)?;
    let responses = service::responses(&mut conn, &actor, assignment_id).await?;
    Ok(Json(FormResponses {
        total_students: responses.total_students,
        missing: responses.missing.into_iter().map(api_name).collect(),
        rows: responses
            .rows
            .into_iter()
            .map(|row| FormResponseRow {
                student_id: row.student.user_id,
                name: row.student.name,
                surname: row.student.surname,
                attempt: row.attempt,
                submitted_at: row.submitted_at,
                is_late: row.is_late,
                answers: row
                    .content
                    .get("answers")
                    .and_then(serde_json::Value::as_object)
                    .cloned()
                    .unwrap_or_default(),
            })
            .collect(),
    }))
}
