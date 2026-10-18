//! `GET /admin/courses` (ADR-0053).

use api_types::{AdminCourse, CourseLine, Page, PageQuery, ProblemDetails};
use axum::Json;
use axum::extract::State;

use super::{api_status, page_params};
use crate::actor::Actor;
use crate::app::AppState;
use crate::error::{ApiError, ValidQuery};
use crate::teaching::service::{self, TeachingError};

/// Every regular course of the institution with its owner, term and student count, paged by id.
/// Admin only.
#[utoipa::path(
    get,
    path = "/admin/courses",
    tag = "admin",
    security(("cookie" = []), ("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "A page of courses", body = Page<AdminCourse>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn list_admin_courses(
    State(state): State<AppState>,
    actor: Actor,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<Page<AdminCourse>>, ApiError> {
    let page = page_params(&query)?;
    let mut conn = state.db.acquire().await.map_err(TeachingError::from)?;
    let courses = service::admin_courses(&mut conn, &actor, page).await?;
    Ok(Json(Page {
        items: courses
            .items
            .into_iter()
            .map(|row| AdminCourse {
                course: CourseLine {
                    course_id: row.course_id,
                    code: row.code,
                    title: row.title,
                },
                status: api_status(row.status),
                owner_id: row.owner_id,
                owner_name: row.owner_name,
                owner_surname: row.owner_surname,
                term_id: row.term_id,
                term_name: row.term_name,
                student_count: row.student_count,
            })
            .collect(),
        next_cursor: courses.next.map(|id| id.to_string()),
    }))
}
