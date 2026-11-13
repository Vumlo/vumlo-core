//! The Buddy's rules (ADR-0070): the agent loop, the daily cap, the chat and its retention, and
//! the proposals. Service functions take a connection and never begin or commit; `answer` takes the
//! pool because it holds no connection while it waits for the model.

use access::Actor;
use auth::{HashedPassword, TemporaryPassword};
use chrono::{DateTime, Utc};
use common::ApiError;
use content::ItemRules;
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::config::Config;
use crate::llm::{self, LlmError, Message, Model};
use crate::model::{ChatMessage, ChatRole, PageContext, Proposal};
use crate::persona::Persona;
use crate::{repo, tools};

/// Rounds of "the model asks for tools, we answer" before the Buddy gives up.
const MAX_STEPS: usize = 6;
/// Changes one message may prepare.
const MAX_PROPOSALS: usize = 5;
/// Earlier messages the model gets to read.
const HISTORY_FOR_MODEL: i64 = 20;
/// Earlier messages the panel shows.
const HISTORY_SHOWN: i64 = 50;
/// The length the table accepts.
const MAX_ANSWER_CHARS: usize = 20_000;
const GIVE_UP: &str = "I could not finish that. Please try a narrower question.";
const PREPARED: &str = "I prepared this. Please check it and confirm.";

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("the model is not configured")]
    NotConfigured,
    #[error(transparent)]
    Llm(#[from] LlmError),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// What one message of the user got: the text, and the changes the Buddy prepared. Nothing of it
/// is stored yet.
#[derive(Debug)]
pub struct Answer {
    pub text: String,
    pub proposals: Vec<tools::Prepared>,
}

/// What one message of the user is answered with: who asked, as what, about what, and when.
pub struct Turn<'a> {
    pub actor: &'a Actor,
    pub persona: Persona,
    pub rules: Option<ItemRules<'a>>,
    pub text: &'a str,
    pub context: &'a PageContext,
    pub now: DateTime<Utc>,
}

/// What a confirm did.
#[derive(Debug)]
pub enum Confirmed {
    Done,
    /// Not theirs, not pending any more, or older than a day.
    NotPending,
}

/// A temporary password made for a confirm that needs one, and its hash. The password goes to the
/// person who confirms in that response and nowhere else.
#[derive(Debug)]
pub struct Credential {
    pub password: TemporaryPassword,
    hash: HashedPassword,
}

#[derive(Debug)]
pub enum ConfirmError {
    /// The core refused the change; its own status and words.
    Refused(ApiError),
    Db(sqlx::Error),
}

impl From<sqlx::Error> for ConfirmError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

pub async fn chat(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<(Vec<ChatMessage>, Vec<Proposal>), sqlx::Error> {
    let messages = repo::recent(conn, user_id, HISTORY_SHOWN).await?;
    let proposals = repo::pending_proposals(conn, user_id).await?;
    Ok((messages, proposals))
}

pub async fn clear(conn: &mut PgConnection, user_id: Uuid) -> Result<(), sqlx::Error> {
    repo::clear(conn, user_id).await
}

/// Takes one of today's messages; `false` when the cap is reached.
pub async fn reserve_message(
    conn: &mut PgConnection,
    user_id: Uuid,
    cap: u32,
) -> Result<bool, sqlx::Error> {
    repo::take_message(conn, user_id, i32::try_from(cap).unwrap_or(i32::MAX)).await
}

pub async fn release_message(conn: &mut PgConnection, user_id: Uuid) -> Result<(), sqlx::Error> {
    repo::release_message(conn, user_id).await
}

pub async fn purge(conn: &mut PgConnection, retention_days: u32) -> Result<u64, sqlx::Error> {
    repo::purge(conn, i32::try_from(retention_days).unwrap_or(i32::MAX)).await
}

/// Stores one exchange: what the user wrote, the answer, and the changes it prepared, together.
/// The handler's transaction.
pub async fn save_exchange(
    conn: &mut PgConnection,
    user_id: Uuid,
    text: &str,
    answer: &Answer,
) -> Result<(ChatMessage, ChatMessage, Vec<Proposal>), sqlx::Error> {
    let user = repo::insert_message(conn, user_id, ChatRole::User, text).await?;
    let said: String = answer.text.chars().take(MAX_ANSWER_CHARS).collect();
    let assistant = repo::insert_message(conn, user_id, ChatRole::Assistant, &said).await?;
    let mut proposals = Vec::new();
    for prepared in &answer.proposals {
        proposals.push(
            repo::insert_proposal(
                conn,
                user_id,
                prepared.tool,
                &prepared.args,
                &prepared.facts,
            )
            .await?,
        );
    }
    Ok((user, assistant, proposals))
}

/// Makes the temporary password a proposal needs, if it needs one: hashing is slow, so it is done
/// before the transaction of [`confirm`] opens. `None` when the proposal needs none or cannot be
/// confirmed any more.
pub async fn credential(
    conn: &mut PgConnection,
    user_id: Uuid,
    id: Uuid,
) -> Result<Option<Credential>, ConfirmError> {
    let Some(args) = repo::pending_args(conn, user_id, id).await? else {
        return Ok(None);
    };
    if !tools::wants_password(&args) {
        return Ok(None);
    }
    let password = TemporaryPassword::generate();
    let hash = auth::hash_password(password.expose())
        .await
        .map_err(|e| ConfirmError::Refused(ApiError::from(e)))?;
    Ok(Some(Credential { password, hash }))
}

/// Runs the change of a proposal the user confirms, as them, in the handler's transaction. The
/// proposal is taken first, so a second confirm finds nothing; a change the core refuses leaves
/// the transaction to be dropped, which puts the proposal back (see [`mark_failed`]).
pub async fn confirm(
    conn: &mut PgConnection,
    actor: &Actor,
    rules: Option<ItemRules<'_>>,
    id: Uuid,
    credential: Option<&Credential>,
) -> Result<Confirmed, ConfirmError> {
    let Some(proposal) = repo::claim_proposal(conn, actor.user_id(), id).await? else {
        return Ok(Confirmed::NotPending);
    };
    tools::execute(
        conn,
        actor,
        rules,
        &proposal.args,
        credential.map(|c| &c.hash),
    )
    .await
    .map_err(ConfirmError::Refused)?;
    Ok(Confirmed::Done)
}

/// A change the core refused is not offered again.
pub async fn mark_failed(
    conn: &mut PgConnection,
    user_id: Uuid,
    id: Uuid,
) -> Result<(), sqlx::Error> {
    repo::fail_proposal(conn, user_id, id).await
}

/// The user turns a proposal down. `false` when there was nothing to turn down.
pub async fn reject(conn: &mut PgConnection, user_id: Uuid, id: Uuid) -> Result<bool, sqlx::Error> {
    repo::reject_proposal(conn, user_id, id).await
}

/// Asks the model, runs the lookups it asks for as `actor`, prepares the changes it asks for, and
/// returns its final text. Nothing is stored here, and no connection is held while the model
/// thinks.
pub async fn answer(
    db: &PgPool,
    config: &Config<'_>,
    turn: Turn<'_>,
) -> Result<Answer, ServiceError> {
    let Turn {
        actor,
        persona,
        rules,
        text,
        context,
        now,
    } = turn;
    let (Some(endpoint), Some(name)) = (config.endpoint(), config.model()) else {
        return Err(ServiceError::NotConfigured);
    };
    let model = Model {
        endpoint,
        name,
        api_key: config.api_key(),
    };
    let history = repo::recent(
        &mut *db.acquire().await?,
        actor.user_id(),
        HISTORY_FOR_MODEL,
    )
    .await?;
    // What the user wrote in this chat: the only text a draft may be made of (see
    // `tools::submitting`).
    let own_words: Vec<String> = history
        .iter()
        .filter(|m| m.role == ChatRole::User)
        .map(|m| m.content.clone())
        .chain(std::iter::once(text.to_owned()))
        .collect();

    let mut messages = vec![Message::System {
        content: persona.system_prompt(now, context),
    }];
    messages.extend(history.into_iter().map(|m| match m.role {
        ChatRole::User => Message::User { content: m.content },
        ChatRole::Assistant => Message::Assistant {
            content: Some(m.content),
            tool_calls: Vec::new(),
        },
    }));
    messages.push(Message::User {
        content: text.to_owned(),
    });

    let specs = tools::specs(persona);
    let mut proposals: Vec<tools::Prepared> = Vec::new();
    for _ in 0..MAX_STEPS {
        let reply = llm::chat(&model, &messages, &specs).await?;
        if reply.tool_calls.is_empty() {
            let said = reply.content.unwrap_or_default();
            let said = said.trim();
            let text = match (said.is_empty(), proposals.is_empty()) {
                (false, _) => said,
                (true, false) => PREPARED,
                (true, true) => GIVE_UP,
            };
            return Ok(Answer {
                text: text.to_owned(),
                proposals,
            });
        }
        messages.push(Message::Assistant {
            content: reply.content,
            tool_calls: reply.tool_calls.clone(),
        });
        for call in reply.tool_calls {
            let mut conn = db.acquire().await?;
            let (name, arguments) = (&call.function.name, &call.function.arguments);
            let result = if tools::is_write(name) {
                if proposals.len() >= MAX_PROPOSALS {
                    json!({"error": "too many changes in one message; ask the user to continue"})
                        .to_string()
                } else {
                    match tools::prepare(
                        &mut conn, actor, rules, &own_words, persona, name, arguments,
                    )
                    .await
                    {
                        Ok(prepared) => {
                            proposals.push(prepared);
                            json!({"proposed": true, "note": "Waiting for the user to confirm. It is not done."})
                                .to_string()
                        }
                        Err(why) => json!({"error": why}).to_string(),
                    }
                }
            } else {
                tools::run(&mut conn, actor, persona, name, arguments, now).await
            };
            messages.push(Message::Tool {
                tool_call_id: call.id,
                content: result,
            });
        }
    }
    Ok(Answer {
        text: GIVE_UP.to_owned(),
        proposals,
    })
}
