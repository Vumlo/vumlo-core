use std::sync::Arc;

use api_types::{ChangePasswordRequest, Page, PageQuery, ProblemDetails, Session, User};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use uuid::Uuid;

use super::{api_kind, api_user};
use crate::actor::{Actor, PendingActor};
use crate::app::AppState;
use crate::error::{ApiError, ValidJson, ValidQuery};
use crate::identity::service::{self, IdentityError, LoginThrottle};
use crate::pagination::PageParams;

/// The signed-in user.
///
/// Also available while a temporary password still has to be changed, so a client can tell
/// why every other endpoint answers 403.
#[utoipa::path(
    get,
    path = "/me",
    tag = "me",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The current user", body = User),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn get_me(
    State(state): State<AppState>,
    PendingActor(actor): PendingActor,
) -> Result<Json<User>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(IdentityError::from)?;
    let user = service::get_me(&mut conn, &actor).await?;
    Ok(Json(api_user(user)))
}

/// Changes the password.
///
/// Clears `must_change_password` and revokes all other sessions of the user; the current
/// session stays. The new password must satisfy the policy (length, strength).
#[utoipa::path(
    post,
    path = "/me/password",
    tag = "me",
    security(("cookie" = []), ("bearer" = [])),
    request_body = ChangePasswordRequest,
    responses(
        (status = 204, description = "Password changed"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Wrong current password or a password that breaks the policy; \
            `errors` explains why", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "Too many wrong current passwords", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn change_password(
    State(state): State<AppState>,
    Extension(throttle): Extension<Arc<LoginThrottle>>,
    PendingActor(actor): PendingActor,
    ValidJson(request): ValidJson<ChangePasswordRequest>,
) -> Result<StatusCode, ApiError> {
    // Read, then hash outside any transaction, then write (ADR-0016).
    let context = {
        let mut conn = state.db.acquire().await.map_err(IdentityError::from)?;
        service::password_change_context(&mut conn, &actor).await?
    };
    let prepared = service::prepare_password_change(
        &throttle,
        &state.hashing,
        context,
        &request.current_password,
        request.new_password,
        state.config.auth.password_min_length,
        &state.config.institution.name,
    )
    .await?;
    let mut tx = state.db.begin().await.map_err(IdentityError::from)?;
    service::change_password(&mut tx, &actor, prepared).await?;
    tx.commit().await.map_err(IdentityError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The user's own sessions, oldest first. `current` marks the one making this request.
#[utoipa::path(
    get,
    path = "/me/sessions",
    tag = "me",
    security(("cookie" = []), ("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "A page of sessions", body = Page<Session>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The password must be changed first", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn list_sessions(
    State(state): State<AppState>,
    actor: Actor,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<Page<Session>>, ApiError> {
    let params = PageParams::new(query.cursor.as_deref(), query.limit)?;
    let mut conn = state.db.acquire().await.map_err(IdentityError::from)?;
    let page = service::list_sessions(&mut conn, &actor, params).await?;
    Ok(Json(Page {
        items: page
            .items
            .into_iter()
            .map(|s| Session {
                id: s.id,
                kind: api_kind(s.kind),
                name: s.name,
                created_at: s.created_at,
                last_seen_at: s.last_seen_at,
                expires_at: s.expires_at,
                current: s.id == actor.session_id(),
            })
            .collect(),
        next_cursor: page.next.map(|id| id.to_string()),
    }))
}

/// Revokes one of the user's own sessions (signing that device out).
#[utoipa::path(
    delete,
    path = "/me/sessions/{id}",
    tag = "me",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Session id")),
    responses(
        (status = 204, description = "Session revoked"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The password must be changed first, or the request \
            origin is not allowed", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such session of this user", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn revoke_session(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(IdentityError::from)?;
    service::revoke_session(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(IdentityError::from)?;
    Ok(StatusCode::NO_CONTENT)
}
