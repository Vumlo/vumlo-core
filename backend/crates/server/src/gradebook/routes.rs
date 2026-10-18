//! HTTP edge of `gradebook`: extractors, DTO mapping, OpenAPI annotations. Writes open the
//! transaction, call exactly one service function and commit; reads use a pooled connection,
//! except the gradebook, which reads three statements from one snapshot.

use api_types::{
    CourseRef, Deadline, GradeAction, GradeActor, GradeCell, GradeHistoryEntry, GradebookCell,
    GradebookColumn, GradebookPage, GradebookRow, MyGrade, MyGrades, Page, PageQuery,
    ProblemDetails, PublishGradesRequest, PublishGradesResponse, PublishStudents, SaveGradeRequest,
    TeachingOverview, ToGrade, UnpublishedContent, UnpublishedGrades, UnpublishedReason,
};
use axum::Json;
use axum::extract::{Path, State};
use serde_json::Value;
use sqlx::types::Decimal;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::model::{self, ColumnStatus, FeedbackInput, GradeInput, Students};
use super::service::{self, GradebookError};
use crate::actor::Actor;
use crate::app::AppState;
use crate::error::{ApiError, ValidJson, ValidQuery};
use crate::pagination::PageParams;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(save_grade))
        .routes(routes!(publish_grades))
        .routes(routes!(get_gradebook))
        .routes(routes!(get_my_grades))
        .routes(routes!(get_grade_history))
        .routes(routes!(get_teaching))
}

impl From<GradebookError> for ApiError {
    fn from(error: GradebookError) -> Self {
        match error {
            GradebookError::Forbidden(policy) => policy.into(),
            GradebookError::Invalid(problems) => Self::Unprocessable {
                detail: "the request has invalid fields".to_owned(),
                errors: problems
                    .into_iter()
                    .map(|problem| api_types::FieldError {
                        field: problem.field.to_owned(),
                        message: problem.message,
                    })
                    .collect(),
            },
            GradebookError::AssignmentNotFound
            | GradebookError::CourseNotFound
            | GradebookError::StudentNotFound => Self::NotFound,
            GradebookError::StudentsNotInCourse(positions) => Self::Unprocessable {
                detail: "the request has invalid fields".to_owned(),
                errors: positions
                    .into_iter()
                    .map(|position| api_types::FieldError {
                        field: format!("students[{position}]"),
                        message: "is not a current student of the course".to_owned(),
                    })
                    .collect(),
            },
            GradebookError::Teaching(e) => e.into(),
            GradebookError::HasGrades => Self::Conflict("this has grades"),
            GradebookError::StudentFormer => {
                Self::Conflict("the student is no longer on the roster, their grades are read-only")
            }
            GradebookError::Events(e) => e.into(),
            GradebookError::Db(e) => Self::from_db("gradebook", &e),
        }
    }
}

/// Points as the API shows them: always two decimals.
fn points(value: Decimal) -> String {
    format!("{value:.2}")
}

fn api_cell(cell: model::Cell) -> GradeCell {
    GradeCell {
        draft_score: cell.draft_score.map(points),
        draft_feedback: cell.draft_feedback,
        published_score: cell.published_score.map(points),
        published_feedback: cell.published_feedback,
        has_unpublished_changes: cell.has_unpublished_changes,
    }
}

fn api_status(status: ColumnStatus) -> api_types::ContentStatus {
    match status {
        ColumnStatus::Draft => api_types::ContentStatus::Draft,
        ColumnStatus::Published => api_types::ContentStatus::Published,
    }
}

fn page_params(query: &PageQuery) -> Result<PageParams, ApiError> {
    Ok(PageParams::new(query.cursor.as_deref(), query.limit)?)
}

/// Saves the draft grade of one student for one assignment. Staff and admins.
///
/// A `PUT` replaces the whole draft cell: both keys are required and `null` clears one. `score`
/// is a decimal string with at most two decimals from 0 to the assignment's `max_points`; a JSON
/// number is rejected. Saving a draft changes nothing the student sees and emits no event.
#[utoipa::path(
    put,
    path = "/assignments/{id}/grades/{student_id}",
    tag = "gradebook",
    security(("cookie" = []), ("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Assignment id"),
        ("student_id" = Uuid, Path, description = "A current student of the course"),
    ),
    request_body = SaveGradeRequest,
    responses(
        (status = 200, description = "The draft cell", body = GradeCell),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the assignment but not grade it",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such assignment, not visible to the caller, or the \
            student is not a current student of its course", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid `score` or `feedback`", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn save_grade(
    State(state): State<AppState>,
    actor: Actor,
    Path((assignment_id, student_id)): Path<(Uuid, Uuid)>,
    ValidJson(request): ValidJson<SaveGradeRequest>,
) -> Result<Json<GradeCell>, ApiError> {
    let input = GradeInput {
        score: request.score,
        feedback: request.feedback.map(|value| match value {
            Value::String(text) => FeedbackInput::Text(text),
            _ => FeedbackInput::NotText,
        }),
    };
    let mut tx = state.db.begin().await.map_err(GradebookError::from)?;
    let cell = service::save_grade(&mut tx, &actor, assignment_id, student_id, input).await?;
    tx.commit().await.map_err(GradebookError::from)?;
    Ok(Json(api_cell(cell)))
}

/// Publishes draft grades: copies draft to published, for every student or a listed set, only
/// where the draft differs from the published value. Staff and admins.
///
/// The body is required: `{"students": "all"}` or `{"students": [id, ...]}`. A listed student
/// must be a current student of the course. Publishing an empty draft over a published grade
/// retracts it. Each student whose published value became non-empty gets one `grade.published`
/// event.
#[utoipa::path(
    post,
    path = "/assignments/{id}/grades/publish",
    tag = "gradebook",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Assignment id")),
    request_body = PublishGradesRequest,
    responses(
        (status = 200, description = "The students whose published value was replaced",
            body = PublishGradesResponse),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the assignment but not publish grades",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such assignment, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A listed student is not a current student of the course",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn publish_grades(
    State(state): State<AppState>,
    actor: Actor,
    Path(assignment_id): Path<Uuid>,
    ValidJson(request): ValidJson<PublishGradesRequest>,
) -> Result<Json<PublishGradesResponse>, ApiError> {
    let students = match request.students {
        PublishStudents::All(_) => Students::All,
        PublishStudents::Ids(ids) => Students::Listed(ids),
    };
    let mut tx = state.db.begin().await.map_err(GradebookError::from)?;
    let published = service::publish_grades(&mut tx, &actor, assignment_id, students).await?;
    tx.commit().await.map_err(GradebookError::from)?;
    Ok(Json(PublishGradesResponse { published }))
}

/// The gradebook of a course, for staff and admins: every assignment as a column in course tree
/// order, and the current students of the course as rows, by surname, name and id, paged by
/// student. A cell carries both slots of the grade; a student never graded in a column has no
/// cell for it.
#[utoipa::path(
    get,
    path = "/courses/{id}/gradebook",
    tag = "gradebook",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id"), PageQuery),
    responses(
        (status = 200, description = "A page of students with the columns", body = GradebookPage),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may see the course but not its gradebook",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn get_gradebook(
    State(state): State<AppState>,
    actor: Actor,
    Path(course_id): Path<Uuid>,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<GradebookPage>, ApiError> {
    let page = page_params(&query)?;
    // Columns, students and cells come from one snapshot, so a publish in between cannot show a
    // cell for a column the page does not list.
    let mut tx = state.db.begin().await.map_err(GradebookError::from)?;
    sqlx::query!("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(GradebookError::from)?;
    let result = service::gradebook(&mut tx, &actor, course_id, page).await?;
    Ok(Json(GradebookPage {
        columns: result
            .columns
            .into_iter()
            .map(|column| GradebookColumn {
                assignment_id: column.assignment_id,
                title: column.title,
                max_points: points(column.max_points),
                is_bonus: column.is_bonus,
                status: api_status(column.status),
            })
            .collect(),
        items: result
            .rows
            .into_iter()
            .map(|row| GradebookRow {
                user_id: row.user_id,
                name: row.name,
                surname: row.surname,
                former: row.former,
                cells: row
                    .cells
                    .into_iter()
                    .map(|(assignment_id, cell)| GradebookCell {
                        assignment_id,
                        cell: api_cell(cell),
                    })
                    .collect(),
            })
            .collect(),
        next_cursor: result.next.map(|id| id.to_string()),
    }))
}

/// The caller's published grades in a course, in course tree order. Students only.
///
/// Only published values are returned, only for assignments the caller may see now. A grade that
/// was never published, or was retracted, does not appear. A draft is never part of this response.
#[utoipa::path(
    get,
    path = "/courses/{id}/my-grades",
    tag = "gradebook",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    responses(
        (status = 200, description = "The published grades", body = MyGrades),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller is not a student of the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn get_my_grades(
    State(state): State<AppState>,
    actor: Actor,
    Path(course_id): Path<Uuid>,
) -> Result<Json<MyGrades>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(GradebookError::from)?;
    let grades = service::my_grades(&mut conn, &actor, course_id).await?;
    Ok(Json(MyGrades {
        items: grades
            .into_iter()
            .map(|grade| MyGrade {
                assignment_id: grade.assignment_id,
                title: grade.title,
                max_points: points(grade.max_points),
                is_bonus: grade.is_bonus,
                score: grade.score.map(points),
                feedback: grade.feedback,
                published_at: grade.published_at,
            })
            .collect(),
    }))
}

/// What the caller has to do across the courses they teach, assist in or own and that are not
/// archived (ADR-0052): the work waiting for a grade, draft grades not published, deadlines coming
/// up and content students cannot read yet. Counts and kinds, not sentences.
#[utoipa::path(
    get,
    path = "/me/teaching",
    tag = "gradebook",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The caller's overview", body = TeachingOverview),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn get_teaching(
    State(state): State<AppState>,
    actor: Actor,
) -> Result<Json<TeachingOverview>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(GradebookError::from)?;
    let overview = service::teaching_overview(&mut conn, &actor, chrono::Utc::now()).await?;
    let refs: std::collections::HashMap<Uuid, CourseRef> = overview
        .courses
        .iter()
        .map(|card| {
            (
                card.course.id,
                CourseRef {
                    course_id: card.course.id,
                    course_code: card.code.clone(),
                    course_title: card.course.title.clone(),
                },
            )
        })
        .collect();
    let course = |id: Uuid| {
        refs.get(&id).cloned().unwrap_or(CourseRef {
            course_id: id,
            course_code: String::new(),
            course_title: String::new(),
        })
    };
    Ok(Json(TeachingOverview {
        to_grade: overview
            .to_grade
            .into_iter()
            .map(|row| ToGrade {
                student_id: row.student_id,
                student_name: row.student_name,
                student_surname: row.student_surname,
                assignment_id: row.assignment_id,
                assignment_title: row.assignment_title,
                course: course(row.course_id),
                submitted_at: row.submitted_at,
                attempt: row.attempt,
                max_attempts: row.max_attempts,
            })
            .collect(),
        unpublished: overview
            .unpublished
            .into_iter()
            .map(|row| UnpublishedGrades {
                assignment_id: row.assignment_id,
                assignment_title: row.assignment_title,
                course: course(row.course_id),
                draft_count: row.draft_count,
                auto_scored: row.auto_scored,
            })
            .collect(),
        deadlines: overview
            .deadlines
            .into_iter()
            .map(|row| Deadline {
                assignment_id: row.assignment_id,
                title: row.title,
                course: course(row.course_id),
                due_at: row.due_at,
                submitted: row.submitted,
                total: row.total,
                not_open: row.not_open,
            })
            .collect(),
        unpublished_content: overview
            .unpublished_content
            .into_iter()
            .map(|row| UnpublishedContent {
                item_id: row.item_id,
                title: row.title,
                course: course(row.course_id),
                reason: if row.edits {
                    UnpublishedReason::Edits
                } else {
                    UnpublishedReason::Draft
                },
            })
            .collect(),
    }))
}

/// The recorded changes of one student's grade on an assignment, newest first. Staff only.
///
/// Every draft save and every publish is a row with the values after it and the person who made it.
/// A student with no grade has an empty history.
#[utoipa::path(
    get,
    path = "/assignments/{id}/grades/{student_id}/history",
    tag = "gradebook",
    security(("cookie" = []), ("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Assignment id"),
        ("student_id" = Uuid, Path, description = "The student"),
        PageQuery,
    ),
    responses(
        (status = 200, description = "A page of history entries", body = Page<GradeHistoryEntry>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not read the gradebook",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such assignment, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn get_grade_history(
    State(state): State<AppState>,
    actor: Actor,
    Path((assignment_id, student_id)): Path<(Uuid, Uuid)>,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<Page<GradeHistoryEntry>>, ApiError> {
    let page = page_params(&query)?;
    let mut conn = state.db.acquire().await.map_err(GradebookError::from)?;
    let history =
        service::grade_history(&mut conn, &actor, assignment_id, student_id, page).await?;
    Ok(Json(Page {
        items: history
            .items
            .into_iter()
            .map(|entry| GradeHistoryEntry {
                id: entry.id,
                action: match entry.action.as_str() {
                    "publish" => GradeAction::Publish,
                    "auto_score" => GradeAction::AutoScore,
                    _ => GradeAction::SaveDraft,
                },
                score: entry.score.map(points),
                feedback: entry.feedback,
                actor: entry.actor.map(|actor| GradeActor {
                    user_id: actor.user_id,
                    name: actor.name,
                    surname: actor.surname,
                }),
                at: entry.at,
            })
            .collect(),
        next_cursor: history.next.map(|id| id.to_string()),
    }))
}
