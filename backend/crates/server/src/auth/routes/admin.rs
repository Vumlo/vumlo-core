use api_types::{
    CreateUserRequest, CreateUserResponse, ListUsersQuery, Page, ProblemDetails,
    TemporaryPasswordResponse, User, UserSummary,
};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use super::{api_user, domain_role, domain_status, no_store};
use crate::access::Actor;
use crate::access::{self, Action};
use crate::app::AppState;
use crate::auth::model::{Email, NewUser, PersonName, TemporaryPassword, UserFilter};
use crate::auth::service::{self, AuthError};
use common::PageParams;
use common::{ApiError, ValidJson, ValidQuery};

/// Refuses non-admins before any work is done on their behalf (parsing, hashing a password).
fn require_admin(actor: &Actor) -> Result<(), AuthError> {
    access::authorize(actor, Action::ManageUsers)?;
    Ok(())
}

/// Lists users, oldest first, optionally filtered. Admin only.
#[utoipa::path(
    get,
    path = "/admin/users",
    tag = "admin",
    security(("cookie" = []), ("bearer" = [])),
    params(ListUsersQuery),
    responses(
        (status = 200, description = "A page of users", body = Page<User>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid filter or cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn list_users(
    State(state): State<AppState>,
    actor: Actor,
    ValidQuery(query): ValidQuery<ListUsersQuery>,
) -> Result<Json<Page<User>>, ApiError> {
    require_admin(&actor)?;
    let page = PageParams::new(query.cursor.as_deref(), query.limit)?;
    let filter = UserFilter {
        role: query.role.map(domain_role),
        status: query.status.map(domain_status),
        email_contains: query.email,
    };
    let mut conn = state.db.acquire().await.map_err(AuthError::from)?;
    let users = service::list_users(&mut conn, &actor, filter, page).await?;
    Ok(Json(Page {
        items: users.items.into_iter().map(api_user).collect(),
        next_cursor: users.next.map(|id| id.to_string()),
    }))
}

/// Creates a user with a generated temporary password. Admin only.
///
/// The password is in this response only; the user must change it at first login.
/// `auth.allowed_domains` does not apply to admin-created users.
#[utoipa::path(
    post,
    path = "/admin/users",
    tag = "admin",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CreateUserRequest,
    responses(
        (status = 201, description = "User created", body = CreateUserResponse),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 409, description = "The email is already in use", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid email or name", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn create_user(
    State(state): State<AppState>,
    actor: Actor,
    ValidJson(request): ValidJson<CreateUserRequest>,
) -> Result<Response, ApiError> {
    require_admin(&actor)?;
    let new = NewUser {
        email: Email::parse(&request.email).map_err(AuthError::from)?,
        name: PersonName::name(&request.name).map_err(AuthError::from)?,
        surname: PersonName::surname(&request.surname).map_err(AuthError::from)?,
        role: domain_role(request.role),
    };
    let temporary = TemporaryPassword::generate();
    let hash = service::hash_password_gated(&state.hashing, temporary.expose()).await?;

    let mut tx = state.db.begin().await.map_err(AuthError::from)?;
    let user = service::admin_create_user(&mut tx, &actor, new, hash).await?;
    tx.commit().await.map_err(AuthError::from)?;

    Ok((
        StatusCode::CREATED,
        [no_store()],
        Json(CreateUserResponse {
            user: api_user(user),
            temporary_password: temporary.expose().to_owned(),
        }),
    )
        .into_response())
}

/// Replaces a user's password with a new temporary one. Admin only.
///
/// The user must change it at next login and is signed out everywhere. The password is in
/// this response only.
#[utoipa::path(
    post,
    path = "/admin/users/{id}/reset-password",
    tag = "admin",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "User id")),
    responses(
        (status = 200, description = "The new temporary password", body = TemporaryPasswordResponse),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such user", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn reset_password(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    require_admin(&actor)?;
    let temporary = TemporaryPassword::generate();
    let hash = service::hash_password_gated(&state.hashing, temporary.expose()).await?;

    let mut tx = state.db.begin().await.map_err(AuthError::from)?;
    service::reset_password(&mut tx, &actor, id, hash).await?;
    tx.commit().await.map_err(AuthError::from)?;

    Ok((
        StatusCode::OK,
        [no_store()],
        Json(TemporaryPasswordResponse {
            temporary_password: temporary.expose().to_owned(),
        }),
    )
        .into_response())
}

/// People by institution role (every status), and how many are disabled. Admin only.
#[utoipa::path(
    get,
    path = "/admin/users/summary",
    tag = "admin",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The counts", body = UserSummary),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Not an admin", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn user_summary(
    State(state): State<AppState>,
    actor: Actor,
) -> Result<Json<UserSummary>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(AuthError::from)?;
    let counts = service::user_summary(&mut conn, &actor).await?;
    Ok(Json(UserSummary {
        all: counts.all,
        student: counts.student,
        teacher: counts.teacher,
        admin: counts.admin,
        disabled: counts.disabled,
    }))
}
