//! The routes of the console that read and retry the mail queue (ADR-0095). A handler takes an
//! `Actor`; the service it calls authorizes: only a console administrator may.

use access::Actor;
use api_types::{
    MailDeadBy, MailMessage, MailMessages, MailMessagesQuery, MailStatus, ProblemDetails,
    RetryMailRequest, RetryMailResponse,
};
use axum::Json;
use axum::extract::State;
use common::{ApiError, ValidJson, ValidQuery};
use vumlo_config::MailTransport;

use crate::admin;
use crate::error::MailError;
use crate::model::{Filter, JournalEntry, MessageState, RetryScope};
use crate::state::MailState;

/// The longest subject the journal shows.
const SUBJECT_CHARS: usize = 80;

fn transport(state: &MailState) -> Option<String> {
    state.settings.as_deref().map(|mail| match mail.transport {
        MailTransport::Smtp { .. } => "smtp".to_owned(),
        MailTransport::File { .. } => "file".to_owned(),
    })
}

fn message(entry: JournalEntry) -> MailMessage {
    let subject = entry.subject.map(|subject| {
        let mut cut: String = subject.chars().take(SUBJECT_CHARS).collect();
        if cut.len() < subject.len() {
            cut.push('…');
        }
        cut
    });
    MailMessage {
        id: entry.id,
        created_at: entry.created_at,
        state: entry.state.as_str().to_owned(),
        attempts: entry.attempts,
        plugin_id: entry.plugin_id,
        kind: entry.kind,
        user_id: entry.user_id,
        subject,
        last_error: entry.last_error,
    }
}

/// What waits, what was sent and what died in the last day, and whether mail is set up at all.
/// Console administrators only.
#[utoipa::path(
    get,
    path = "/admin/mail/status",
    tag = "admin",
    security(("console" = [])),
    responses(
        (status = 200, description = "The counts", body = MailStatus),
        (status = 403, description = "Not a console administrator", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub(crate) async fn get_status(
    actor: Actor,
    State(state): State<MailState>,
) -> Result<Json<MailStatus>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(MailError::from)?;
    let status = admin::admin_status(&mut conn, &actor).await?;
    Ok(Json(MailStatus {
        configured: state.is_configured(),
        transport: transport(&state),
        pending: status.pending,
        sent_24h: status.sent_24h,
        dead_24h: status.dead_24h,
        cancelled_24h: status.cancelled_24h,
        dead_now: status.dead_now,
        oldest_pending: status.oldest_pending,
        dead_by: status
            .dead_by
            .into_iter()
            .map(|(plugin_id, kind, dead)| MailDeadBy {
                plugin_id,
                kind,
                dead,
            })
            .collect(),
    }))
}

/// The newest messages of the journal, by state, plugin, kind and person. A person is an id, never
/// an address. Console administrators only.
#[utoipa::path(
    get,
    path = "/admin/mail/messages",
    tag = "admin",
    security(("console" = [])),
    params(MailMessagesQuery),
    responses(
        (status = 200, description = "The newest messages first", body = MailMessages),
        (status = 403, description = "Not a console administrator", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "A state that does not exist", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub(crate) async fn list_messages(
    actor: Actor,
    State(state): State<MailState>,
    ValidQuery(query): ValidQuery<MailMessagesQuery>,
) -> Result<Json<MailMessages>, ApiError> {
    let message_state = query
        .state
        .as_deref()
        .map(|name| {
            name.parse::<MessageState>().map_err(|_| {
                ApiError::invalid_field("state", "must be pending, sent, dead or cancelled")
            })
        })
        .transpose()?;
    let filter = Filter {
        state: message_state,
        plugin_id: query.plugin_id,
        kind: query.kind,
        user_id: query.user_id,
        hours: query.hours,
    };
    let mut conn = state.db.acquire().await.map_err(MailError::from)?;
    let items = admin::admin_list(&mut conn, &actor, &filter, query.limit.unwrap_or(50)).await?;
    Ok(Json(MailMessages {
        items: items.into_iter().map(message).collect(),
    }))
}

/// Puts dead mail back in the queue: one message by `id`, or the dead mail of a plugin, or all of
/// it. A message whose address is already erased cannot be sent and stays dead. Console
/// administrators only.
#[utoipa::path(
    post,
    path = "/admin/mail/retry",
    tag = "admin",
    security(("console" = [])),
    request_body = RetryMailRequest,
    responses(
        (status = 200, description = "How many were put back and how many could not be", body = RetryMailResponse),
        (status = 403, description = "Not a console administrator", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub(crate) async fn retry(
    actor: Actor,
    State(state): State<MailState>,
    ValidJson(request): ValidJson<RetryMailRequest>,
) -> Result<Json<RetryMailResponse>, ApiError> {
    let scope = match request.id {
        Some(id) => RetryScope::One(id),
        None => RetryScope::Dead {
            plugin_id: request.plugin_id,
        },
    };
    let mut tx = state.db.begin().await.map_err(MailError::from)?;
    let retried = admin::admin_retry(&mut tx, &actor, &scope).await?;
    tx.commit().await.map_err(MailError::from)?;
    Ok(Json(RetryMailResponse {
        requeued: i64::try_from(retried.requeued.len()).unwrap_or(i64::MAX),
        erased: i64::try_from(retried.erased.len()).unwrap_or(i64::MAX),
    }))
}
