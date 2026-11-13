//! `GET /admin/courses` (ADR-0053).

use api_types::{AdminCourse, AllCoursesQuery, CourseLine, Page, ProblemDetails};
use axum::Json;
use axum::extract::State;

use super::{access_status, api_status};
use crate::TeachingState;
use crate::model::AdminCourseFilter;
use crate::service::{self, TeachingError};
use access::Actor;
use common::{ApiError, PageParams, ValidQuery};

/// Every regular course of the institution with its owner, term and student count, newest first,
/// narrowed by term, status and a search text. Admin only.
#[utoipa::path(
    get,
    path = "/admin/courses",
    tag = "admin",
    security(("cookie" = []), ("bearer" = [])),
    params(AllCoursesQuery),
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
    State(state): State<TeachingState>,
    actor: Actor,
    ValidQuery(query): ValidQuery<AllCoursesQuery>,
) -> Result<Json<Page<AdminCourse>>, ApiError> {
    let page = PageParams::new(query.cursor.as_deref(), query.limit)?;
    let filter = AdminCourseFilter {
        term_id: query.term_id,
        status: query.status.map(access_status),
        q: query.q,
    };
    let mut conn = state.db.acquire().await.map_err(TeachingError::from)?;
    let courses = service::admin_courses(&mut conn, &actor, filter, page).await?;
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
