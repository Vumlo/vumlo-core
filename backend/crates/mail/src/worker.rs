//! The mail worker, the outbox worker's twin (ADR-0084): every 2 s it claims 20 due messages with a
//! 5-minute lease, sends each with no transaction open, and marks it. Delivery is at-least-once.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sqlx::PgPool;

use crate::compose::{self, Footer};
use crate::error::MailError;
use crate::mailer::{Mailer, Outgoing};
use crate::model::MessageState;
use crate::optout::OptOut;
use crate::repo::{self, Claimed};
use crate::service::{self, DEDUPE_DAYS};
use vumlo_config::{Language, MailSettings};

/// Messages claimed per round.
const BATCH: i64 = 20;
/// How long a claimed message belongs to the worker that claimed it.
const LEASE: Duration = Duration::from_secs(5 * 60);
/// How long the worker sleeps when there was nothing to do.
const POLL: Duration = Duration::from_secs(2);
/// How often it erases and prunes.
const SWEEP_EVERY: Duration = Duration::from_secs(60);

/// What the worker needs besides the pool and the transport.
#[derive(Debug, Clone)]
pub struct Context {
    /// The language of the footer and of the answers of the opt-out routes.
    pub language: Language,
    pub institution_name: String,
    /// Days an address is kept after a plugin message ends.
    pub retention_days: i32,
    /// Days the journal is kept.
    pub journal_days: i32,
}

impl Context {
    pub fn new(settings: &MailSettings, language: Language, institution_name: &str) -> Self {
        Self {
            language,
            institution_name: institution_name.to_owned(),
            retention_days: i32::try_from(settings.retention_days).unwrap_or(i32::MAX),
            journal_days: i32::try_from(settings.journal_days).unwrap_or(i32::MAX),
        }
    }
}

/// Claims one batch and sends it. Returns how many messages it claimed.
pub async fn process_once(
    pool: &PgPool,
    mailer: &Mailer,
    context: &Context,
) -> Result<usize, MailError> {
    let claimed = {
        let mut conn = pool.acquire().await?;
        repo::claim(&mut conn, BATCH, LEASE.as_secs_f64()).await?
    };
    let count = claimed.len();
    let mut key: Option<Vec<u8>> = None;
    for message in claimed {
        deliver(pool, mailer, context, &mut key, message).await?;
    }
    Ok(count)
}

enum Outcome {
    Sent,
    Cancelled,
    Dead(String),
    Retry(String),
}

async fn deliver(
    pool: &PgPool,
    mailer: &Mailer,
    context: &Context,
    key: &mut Option<Vec<u8>>,
    message: Claimed,
) -> Result<(), MailError> {
    let attempts = message.attempts;
    let (id, user_id) = (message.id, message.user_id);
    let erase_days = if message.plugin_id.is_some() {
        context.retention_days
    } else {
        0
    };
    let outcome = decide(pool, mailer, context, key, message).await?;
    let mut conn = pool.acquire().await?;
    match outcome {
        Outcome::Sent => {
            repo::finish(&mut conn, id, MessageState::Sent, None, erase_days).await?;
        }
        Outcome::Cancelled => {
            repo::finish(&mut conn, id, MessageState::Cancelled, None, erase_days).await?;
        }
        Outcome::Dead(class) => {
            tracing::error!(message_id = %id, %user_id, attempts, class = %class, "mail given up");
            repo::finish(&mut conn, id, MessageState::Dead, Some(&class), erase_days).await?;
        }
        Outcome::Retry(class) => {
            tracing::warn!(message_id = %id, %user_id, attempts, class = %class, "mail failed, will retry");
            repo::schedule_retry(
                &mut conn,
                id,
                &class,
                events::backoff(attempts).as_secs_f64(),
            )
            .await?;
        }
    }
    Ok(())
}

/// Builds the message and sends it, with no transaction open. The class of a failure is all that is
/// kept of it.
async fn decide(
    pool: &PgPool,
    mailer: &Mailer,
    context: &Context,
    key: &mut Option<Vec<u8>>,
    message: Claimed,
) -> Result<Outcome, MailError> {
    if message.expired {
        return Ok(Outcome::Dead("expired".to_owned()));
    }
    let (Some(address), Some(subject), Some(text)) =
        (&message.to_address, &message.subject, &message.body)
    else {
        return Ok(Outcome::Dead("erased".to_owned()));
    };
    let name = message.to_name.as_deref().unwrap_or_default();
    let result = match &message.plugin_id {
        None => {
            mailer
                .send(Outgoing {
                    name,
                    address,
                    subject,
                    body: text.clone(),
                    plugin: None,
                })
                .await
        }
        Some(plugin_id) => {
            let mut conn = pool.acquire().await?;
            // The person may have opted out since the mail was queued.
            if repo::is_opted_out(&mut conn, message.user_id, plugin_id, &message.kind).await? {
                return Ok(Outcome::Cancelled);
            }
            let signing = match key {
                Some(key) => key.clone(),
                None => {
                    let made = service::signing_key(&mut conn).await?;
                    *key = Some(made.clone());
                    made
                }
            };
            drop(conn);
            let opt_out = OptOut {
                user_id: message.user_id,
                plugin_id: plugin_id.clone(),
                kind: message.kind.clone(),
            };
            let Some(token) = service::token(&signing, &opt_out) else {
                return Ok(Outcome::Dead("build".to_owned()));
            };
            let opt_out_link = mailer.link(&format!("/api/v1/mail/opt-out/{token}"));
            let link = message.link_path.as_deref().map(|path| mailer.link(path));
            let body = compose::plugin_body(
                text,
                link.as_deref(),
                &Footer {
                    language: context.language,
                    institution: &context.institution_name,
                    plugin_id,
                    kind: &message.kind,
                    opt_out_link: &opt_out_link,
                },
            );
            mailer
                .send(Outgoing {
                    name,
                    address,
                    subject,
                    body,
                    plugin: Some((plugin_id, &opt_out_link)),
                })
                .await
        }
    };
    Ok(match result {
        Ok(()) => Outcome::Sent,
        Err(error) if error.is_permanent() => Outcome::Dead(error.class().to_owned()),
        Err(error) => Outcome::Retry(error.class().to_owned()),
    })
}

/// Erases the addresses that are due, the text of batches that hold none, and prunes the journal
/// and the dedupe table (ADR-0084).
pub async fn sweep(pool: &PgPool, context: &Context) -> Result<(), MailError> {
    let mut conn = pool.acquire().await?;
    repo::erase_addresses(&mut conn).await?;
    repo::erase_batch_content(&mut conn).await?;
    repo::delete_old_batches(&mut conn, context.journal_days).await?;
    repo::delete_old_dedupe(&mut conn, DEDUPE_DAYS).await?;
    Ok(())
}

/// Sends the queue until the process ends. ponytail: one message at a time and no graceful stop (a
/// message cut off by a shutdown is sent again after its lease); add a concurrency limit and a stop
/// signal when throughput or deploys need them.
pub async fn run_worker(pool: PgPool, mailer: Arc<Mailer>, context: Context) {
    let mut last_sweep = Instant::now()
        .checked_sub(SWEEP_EVERY)
        .unwrap_or_else(Instant::now);
    loop {
        if last_sweep.elapsed() >= SWEEP_EVERY {
            last_sweep = Instant::now();
            if let Err(error) = sweep(&pool, &context).await {
                tracing::error!(%error, "the mail sweep failed");
            }
        }
        match process_once(&pool, &mailer, &context).await {
            Ok(n) if n as i64 >= BATCH => continue,
            Ok(_) => {}
            Err(error) => tracing::error!(%error, "the mail worker failed a round"),
        }
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests;
