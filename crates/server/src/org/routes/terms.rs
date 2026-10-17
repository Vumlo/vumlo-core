use api_types::{CreateTermRequest, Page, PageQuery, ProblemDetails, Term, UpdateTermRequest};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use uuid::Uuid;

use super::{api_page, page_params};
use crate::actor::Actor;
use crate::app::AppState;
use crate::error::{ApiError, ValidJson, ValidQuery};
use crate::org::model::{self, TermUpdate};
use crate::org::service::{self, OrgError};

fn api_term(term: model::Term) -> Term {
    Term {
        id: term.id,
        name: term.name,
        starts_on: term.starts_on,
        ends_on: term.ends_on,
    }
}

/// Creates a term. Admin only.
#[utoipa::path(
    post,
    path = "/terms",
    tag = "terms",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CreateTermRequest,
    responses(
        (status = 201, description = "The new term", body = Term),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 409, description = "The name is taken", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid name, or ends_on before starts_on",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn create_term(
    State(state): State<AppState>,
    actor: Actor,
    ValidJson(request): ValidJson<CreateTermRequest>,
) -> Result<(StatusCode, Json<Term>), ApiError> {
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    let term = service::create_term(
        &mut tx,
        &actor,
        &request.name,
        request.starts_on,
        request.ends_on,
    )
    .await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok((StatusCode::CREATED, Json(api_term(term))))
}

/// The terms, in creation order. Any signed-in user.
#[utoipa::path(
    get,
    path = "/terms",
    tag = "terms",
    security(("cookie" = []), ("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "A page of terms", body = Page<Term>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn list_terms(
    State(state): State<AppState>,
    actor: Actor,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<Page<Term>>, ApiError> {
    let page = page_params(&query)?;
    let mut conn = state.db.acquire().await.map_err(OrgError::from)?;
    let terms = service::list_terms(&mut conn, &actor, page).await?;
    Ok(Json(api_page(terms, api_term)))
}

/// One term. Any signed-in user.
#[utoipa::path(
    get,
    path = "/terms/{id}",
    tag = "terms",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Term id")),
    responses(
        (status = 200, description = "The term", body = Term),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such term", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn get_term(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<Term>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(OrgError::from)?;
    Ok(Json(api_term(
        service::get_term(&mut conn, &actor, id).await?,
    )))
}

/// Changes the name or the dates of a term. Admin only.
///
/// Only the fields present change; the dates are checked together after the merge.
#[utoipa::path(
    patch,
    path = "/terms/{id}",
    tag = "terms",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Term id")),
    request_body = UpdateTermRequest,
    responses(
        (status = 200, description = "The updated term", body = Term),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such term", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 409, description = "The name is taken", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid name, or ends_on before starts_on",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn update_term(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<UpdateTermRequest>,
) -> Result<Json<Term>, ApiError> {
    let update = TermUpdate {
        name: request.name,
        starts_on: request.starts_on,
        ends_on: request.ends_on,
    };
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    let term = service::update_term(&mut tx, &actor, id, update).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok(Json(api_term(term)))
}

/// Deletes a term that no course uses. Admin only.
#[utoipa::path(
    delete,
    path = "/terms/{id}",
    tag = "terms",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Term id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such term", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 409, description = "A course uses the term", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn delete_term(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(OrgError::from)?;
    service::delete_term(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(OrgError::from)?;
    Ok(StatusCode::NO_CONTENT)
}
