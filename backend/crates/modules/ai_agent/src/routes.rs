//! HTTP: extractors and the mapping between DTOs and the domain (ADR-0070). Every route first asks
//! which kind of user this is; where the Buddy is not switched on for them it is a `404`.

use std::collections::BTreeMap;

use access::Actor;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use common::ApiError;
use module_api::ModuleState;
use serde::{Deserialize, Serialize};
use sqlx::Postgres;
use sqlx::pool::PoolConnection;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::config::Config;
use crate::model::{ChatMessage, ChatRole, PageContext, Proposal};
use crate::persona::Persona;
use crate::service::{self, ConfirmError, Confirmed, ServiceError};

const MAX_TEXT_CHARS: usize = 4000;
const MAX_PAGE_CHARS: usize = 200;

pub fn router() -> OpenApiRouter<ModuleState> {
    OpenApiRouter::new()
        .routes(routes!(get_chat, clear_chat))
        .routes(routes!(post_message))
        .routes(routes!(confirm_proposal))
        .routes(routes!(reject_proposal))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MessageDto {
    pub id: Uuid,
    /// `user` or `assistant`.
    pub role: &'static str,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

impl From<ChatMessage> for MessageDto {
    fn from(m: ChatMessage) -> Self {
        Self {
            id: m.id,
            role: match m.role {
                ChatRole::User => "user",
                ChatRole::Assistant => "assistant",
            },
            content: m.content,
            created_at: m.created_at,
        }
    }
}

/// A change the Buddy prepared. Nothing has happened yet: it happens when the user confirms.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProposalDto {
    pub id: Uuid,
    /// `set_grade`, `publish_grades` or `set_deadline`. The client words the card from this and
    /// `facts`, in the user's language.
    pub tool: String,
    /// What the card shows, read from the service as the user: `assignment`, `student`, `score`,
    /// `max_points`, `feedback`, `due_at` (an RFC 3339 time, or empty for "no deadline").
    pub facts: BTreeMap<String, String>,
    pub created_at: DateTime<Utc>,
}

impl From<Proposal> for ProposalDto {
    fn from(p: Proposal) -> Self {
        Self {
            id: p.id,
            tool: p.tool,
            facts: serde_json::from_str(&p.facts).unwrap_or_default(),
            created_at: p.created_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChatResponse {
    /// What to show before the first message: where the messages go. Null when the admin set none.
    pub notice: Option<String>,
    /// The last messages, oldest first.
    pub messages: Vec<MessageDto>,
    /// Changes waiting for the user's confirm.
    pub proposals: Vec<ProposalDto>,
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct PageContextDto {
    /// The page the user has open, such as `/courses`. At most 200 characters.
    pub page: Option<String>,
    pub course_id: Option<Uuid>,
    pub item_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewMessage {
    /// 1 to 4000 characters.
    pub text: String,
    #[serde(default)]
    pub context: PageContextDto,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ExchangeResponse {
    pub user: MessageDto,
    pub assistant: MessageDto,
    /// What the Buddy prepared in this answer, if anything.
    pub proposals: Vec<ProposalDto>,
}

fn db(error: sqlx::Error) -> ApiError {
    ApiError::from_db("ai_agent", &error)
}

async fn connection(state: &ModuleState) -> Result<PoolConnection<Postgres>, ApiError> {
    state.db.acquire().await.map_err(db)
}

fn api_error(error: ServiceError) -> ApiError {
    match error {
        ServiceError::Db(e) => db(e),
        ServiceError::NotConfigured => {
            tracing::error!("ai_agent: the endpoint or the model is not set");
            ApiError::ServiceUnavailable("the buddy is not available right now")
        }
        // The cause was logged where it met the model; the user gets the same words for all.
        ServiceError::Llm(_) => {
            ApiError::ServiceUnavailable("the buddy is not available right now")
        }
    }
}

/// What a confirm answers. Only a password reset has anything to say.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConfirmResponse {
    /// The temporary password of a reset. It is in this response only, never stored or shown to
    /// the model; the person must change it at their next sign-in.
    pub temporary_password: Option<String>,
}

fn confirm_failure(error: ConfirmError) -> ApiError {
    match error {
        ConfirmError::Refused(refusal) => refusal,
        ConfirmError::Db(e) => db(e),
    }
}

fn context(dto: PageContextDto) -> Result<PageContext, ApiError> {
    let page = dto
        .page
        .map(|p| p.chars().filter(|c| !c.is_control()).collect::<String>())
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty());
    if page
        .as_ref()
        .is_some_and(|p| p.chars().count() > MAX_PAGE_CHARS)
    {
        return Err(ApiError::invalid_field(
            "context.page",
            format!("at most {MAX_PAGE_CHARS} characters"),
        ));
    }
    Ok(PageContext {
        page,
        course_id: dto.course_id,
        item_id: dto.item_id,
    })
}

/// The caller's own chat and the notice to show first.
#[utoipa::path(get, path = "/chat", responses((status = 200, body = ChatResponse)))]
async fn get_chat(
    actor: Actor,
    State(state): State<ModuleState>,
) -> Result<Json<ChatResponse>, ApiError> {
    Persona::of(&state, &actor)?;
    let mut conn = connection(&state).await?;
    let (messages, proposals) = service::chat(&mut conn, actor.user_id())
        .await
        .map_err(db)?;
    Ok(Json(ChatResponse {
        notice: Config::new(&state.settings).notice().map(str::to_owned),
        messages: messages.into_iter().map(MessageDto::from).collect(),
        proposals: proposals.into_iter().map(ProposalDto::from).collect(),
    }))
}

/// Deletes the caller's chat.
#[utoipa::path(delete, path = "/chat", responses((status = 204)))]
async fn clear_chat(
    actor: Actor,
    State(state): State<ModuleState>,
) -> Result<StatusCode, ApiError> {
    Persona::of(&state, &actor)?;
    let mut conn = connection(&state).await?;
    service::clear(&mut conn, actor.user_id())
        .await
        .map_err(db)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Sends a message and returns the Buddy's answer. `429` once the daily cap is used, `503` while
/// the model cannot be reached.
#[utoipa::path(
    post,
    path = "/messages",
    request_body = NewMessage,
    responses((status = 200, body = ExchangeResponse))
)]
async fn post_message(
    actor: Actor,
    State(state): State<ModuleState>,
    Json(new): Json<NewMessage>,
) -> Result<Json<ExchangeResponse>, ApiError> {
    let persona = Persona::of(&state, &actor)?;
    let text = new.text.trim();
    if text.is_empty() || text.chars().count() > MAX_TEXT_CHARS {
        return Err(ApiError::invalid_field(
            "text",
            format!("1 to {MAX_TEXT_CHARS} characters"),
        ));
    }
    let context = context(new.context)?;
    let config = Config::new(&state.settings);
    let user_id = actor.user_id();

    let reserved = {
        let mut conn = connection(&state).await?;
        service::reserve_message(&mut conn, user_id, config.daily_cap())
            .await
            .map_err(db)?
    };
    if !reserved {
        return Err(ApiError::TooManyRequests {
            retry_after_secs: None,
        });
    }

    // Slow, and outside any transaction (AGENTS §4).
    let turn = service::Turn {
        actor: &actor,
        persona,
        rules: crate::tools::rules(&state.institution),
        text,
        context: &context,
        now: Utc::now(),
    };
    let answer = match service::answer(&state.db, &config, turn).await {
        Ok(answer) => answer,
        Err(error) => {
            if let Ok(mut conn) = state.db.acquire().await {
                // A message the user could not use is not counted against them.
                let _ = service::release_message(&mut conn, user_id).await;
            }
            return Err(api_error(error));
        }
    };

    let mut tx = state.db.begin().await.map_err(db)?;
    let (user, assistant, proposals) = service::save_exchange(&mut tx, user_id, text, &answer)
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(Json(ExchangeResponse {
        user: user.into(),
        assistant: assistant.into(),
        proposals: proposals.into_iter().map(ProposalDto::from).collect(),
    }))
}

/// Makes the change of a proposal, as the caller, once. The core's own rules apply, and a change it
/// refuses answers with its own status (`403`, `409`, `422`) and leaves nothing changed. `409` when
/// the proposal was already handled, is not the caller's, or is older than a day. The answer is
/// `no-store`: a password reset puts the temporary password in it.
#[utoipa::path(
    post,
    path = "/proposals/{id}/confirm",
    params(("id" = Uuid, Path, description = "Proposal id")),
    responses((status = 200, description = "The change was made", body = ConfirmResponse))
)]
async fn confirm_proposal(
    actor: Actor,
    State(state): State<ModuleState>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    Persona::of(&state, &actor)?;
    // Hashing a temporary password is slow, so it is done before the transaction opens.
    let credential = {
        let mut conn = connection(&state).await?;
        service::credential(&mut conn, actor.user_id(), id)
            .await
            .map_err(confirm_failure)?
    };
    let mut tx = state.db.begin().await.map_err(db)?;
    let rules = crate::tools::rules(&state.institution);
    match service::confirm(&mut tx, &actor, rules, id, credential.as_ref()).await {
        Ok(Confirmed::Done) => {
            tx.commit().await.map_err(db)?;
            let body = ConfirmResponse {
                temporary_password: credential.map(|c| c.password.expose().to_owned()),
            };
            Ok(([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response())
        }
        Ok(Confirmed::NotPending) => Err(ApiError::Conflict(
            "this change was already handled, or it has expired",
        )),
        Err(ConfirmError::Refused(refusal)) => {
            // Roll the change back, then remember that it was refused.
            drop(tx);
            if let Ok(mut conn) = state.db.acquire().await {
                let _ = service::mark_failed(&mut conn, actor.user_id(), id).await;
            }
            Err(refusal)
        }
        Err(ConfirmError::Db(e)) => Err(db(e)),
    }
}

/// Turns a proposal down; nothing changes. `409` when there is nothing to turn down.
#[utoipa::path(
    post,
    path = "/proposals/{id}/reject",
    params(("id" = Uuid, Path, description = "Proposal id")),
    responses((status = 204, description = "The proposal was dropped"))
)]
async fn reject_proposal(
    actor: Actor,
    State(state): State<ModuleState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    Persona::of(&state, &actor)?;
    let mut conn = connection(&state).await?;
    if service::reject(&mut conn, actor.user_id(), id)
        .await
        .map_err(db)?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::Conflict(
            "this change was already handled, or it has expired",
        ))
    }
}
