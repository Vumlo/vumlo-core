//! Archive, restore and the search for people to add (ADR-0048). The header, deletion, templates
//! and the people list are flows over several modules: `learning::courses`.

use api_types::{Candidate, Candidates, CandidatesQuery, Course, ProblemDetails};
use axum::Json;
use axum::extract::{Path, State};
use uuid::Uuid;

use super::api_course;
use crate::LearningState;
use crate::teaching::model;
use crate::teaching::service::{self, TeachingError};
use access::Actor;
use common::Role;
use common::{ApiError, ValidQuery};

fn api_candidate(candidate: model::Candidate) -> Candidate {
    Candidate {
        user_id: candidate.user_id,
        name: candidate.name,
        surname: candidate.surname,
        email: candidate.email,
        role: match candidate.role {
            Role::Admin => api_types::Role::Admin,
            Role::Teacher => api_types::Role::Teacher,
            Role::Student => api_types::Role::Student,
        },
    }
}

/// Archives a course: it stays readable, for students too, and takes no more changes. Owner,
/// course teacher or admin. Idempotent.
#[utoipa::path(
    post,
    path = "/courses/{id}/archive",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    responses(
        (status = 200, description = "The archived course", body = Course),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not archive this course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn archive_course(
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<Course>, ApiError> {
    let mut tx = state.db.begin().await.map_err(TeachingError::from)?;
    let course = service::archive_course(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(TeachingError::from)?;
    Ok(Json(api_course(course)))
}

/// Takes a course out of the archive. It is published again, also when it was a draft before.
/// Owner, course teacher or admin. Idempotent.
#[utoipa::path(
    post,
    path = "/courses/{id}/restore",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    responses(
        (status = 200, description = "The restored course", body = Course),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not restore this course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn restore_course(
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<Course>, ApiError> {
    let mut tx = state.db.begin().await.map_err(TeachingError::from)?;
    let course = service::restore_course(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(TeachingError::from)?;
    Ok(Json(api_course(course)))
}

/// Active people who are not on the course and match `q` in their name, surname or email: at
/// most 20, ordered by surname. Whoever manages the roster (assistants included).
#[utoipa::path(
    get,
    path = "/courses/{id}/candidates",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id"), CandidatesQuery),
    responses(
        (status = 200, description = "Up to 20 people", body = Candidates),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not manage this roster",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A search text shorter than two characters",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn get_candidates(
    State(state): State<LearningState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidQuery(query): ValidQuery<CandidatesQuery>,
) -> Result<Json<Candidates>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(TeachingError::from)?;
    let found = service::course_candidates(&mut conn, &actor, id, &query.q).await?;
    Ok(Json(Candidates {
        items: found.into_iter().map(api_candidate).collect(),
    }))
}
