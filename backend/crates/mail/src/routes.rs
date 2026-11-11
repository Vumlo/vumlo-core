//! The only routes of the layer: the one-click opt-out and its undo (ADR-0086). No session, no
//! CSRF header and no body: the one-click `POST` of a mail client sends a form that is ignored.

use axum::extract::{FromRef, Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use common::ApiError;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::compose;
use crate::service;
use crate::state::MailState;

/// The routes of the layer, to be merged under `/api/v1`.
pub fn router<S>() -> OpenApiRouter<S>
where
    S: Clone + Send + Sync + 'static,
    MailState: FromRef<S>,
{
    OpenApiRouter::new()
        .routes(routes!(opt_out_get, opt_out_post))
        .routes(routes!(opt_in_get, opt_in_post))
}

fn text(body: String) -> Response {
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        body,
    )
        .into_response()
}

async fn apply(state: &MailState, token: &str, opt_out: bool) -> Result<Response, ApiError> {
    let mut conn = state.db.acquire().await.map_err(crate::MailError::from)?;
    let Some(named) = service::apply_token(&mut conn, token, opt_out).await? else {
        return Err(ApiError::NotFound);
    };
    Ok(text(if opt_out {
        compose::opted_out_answer(
            state.language,
            &named.plugin_id,
            &format!("{}/api/v1/mail/opt-in/{token}", state.link_base),
        )
    } else {
        compose::opted_in_answer(state.language, &named.plugin_id)
    }))
}

/// Stops the messages of one kind from one plugin for the person the link was made for.
///
/// The link is signed and never expires. A mail scanner that opens links will opt people out, and
/// the answer holds the link that undoes it.
#[utoipa::path(
    get,
    path = "/mail/opt-out/{token}",
    tag = "mail",
    params(("token" = String, Path, description = "The signed token from the footer of a mail")),
    responses(
        (status = 200, description = "Opted out", content_type = "text/plain", body = String),
        (status = 404, description = "Not a link this server made", content_type = "application/problem+json"),
    )
)]
async fn opt_out_get(
    State(state): State<MailState>,
    Path(token): Path<String>,
) -> Result<Response, ApiError> {
    apply(&state, &token, true).await
}

/// The one-click form of the opt-out (RFC 8058): the same as `GET`.
#[utoipa::path(
    post,
    path = "/mail/opt-out/{token}",
    tag = "mail",
    params(("token" = String, Path, description = "The signed token from the footer of a mail")),
    responses(
        (status = 200, description = "Opted out", content_type = "text/plain", body = String),
        (status = 404, description = "Not a link this server made", content_type = "application/problem+json"),
    )
)]
async fn opt_out_post(
    State(state): State<MailState>,
    Path(token): Path<String>,
) -> Result<Response, ApiError> {
    apply(&state, &token, true).await
}

/// Undoes an opt-out made with the same token.
#[utoipa::path(
    get,
    path = "/mail/opt-in/{token}",
    tag = "mail",
    params(("token" = String, Path, description = "The token of the opt-out link")),
    responses(
        (status = 200, description = "Opted in again", content_type = "text/plain", body = String),
        (status = 404, description = "Not a link this server made", content_type = "application/problem+json"),
    )
)]
async fn opt_in_get(
    State(state): State<MailState>,
    Path(token): Path<String>,
) -> Result<Response, ApiError> {
    apply(&state, &token, false).await
}

/// The same as `GET /mail/opt-in/{token}`.
#[utoipa::path(
    post,
    path = "/mail/opt-in/{token}",
    tag = "mail",
    params(("token" = String, Path, description = "The token of the opt-out link")),
    responses(
        (status = 200, description = "Opted in again", content_type = "text/plain", body = String),
        (status = 404, description = "Not a link this server made", content_type = "application/problem+json"),
    )
)]
async fn opt_in_post(
    State(state): State<MailState>,
    Path(token): Path<String>,
) -> Result<Response, ApiError> {
    apply(&state, &token, false).await
}
