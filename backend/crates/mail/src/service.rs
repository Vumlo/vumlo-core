//! The rules of the queue (ADR-0084, ADR-0085). Every function takes the caller's connection and never
//! begins a transaction: the mail is queued in the transaction of whatever asked for it.

use std::collections::{HashMap, HashSet};

use rand::Rng;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::MailError;
use crate::model::{
    AccountMail, Filter, JournalEntry, Limits, PluginMail, PluginOutcome, Retried, RetryScope,
    Skipped, Status,
};
use crate::optout::{self, OptOut};
use crate::repo;

const HOUR_SECS: f64 = 3600.0;
const DAY_SECS: f64 = 86_400.0;
/// How long a `dedupe_key` is remembered (ADR-0085).
pub(crate) const DEDUPE_DAYS: i32 = 30;

/// Seconds until the oldest of a window's entries leaves it, at least one.
fn retry_after(window_secs: f64, oldest_age: f64) -> u64 {
    (window_secs - oldest_age).ceil().max(1.0) as u64
}

/// Whether a message answering the outbox event `origin_id` is queued already. A handler asks
/// before it mints a token, so a rerun does not mint a second one.
pub async fn origin_queued(conn: &mut PgConnection, origin_id: Uuid) -> Result<bool, MailError> {
    Ok(repo::origin_queued(conn, origin_id).await?)
}

/// Queues one account mail (verification, password reset). Returns `false` when `origin_id` was
/// queued before. It is never opt-out-able and meets no plugin limit.
pub async fn enqueue_account(
    conn: &mut PgConnection,
    mail: AccountMail,
) -> Result<bool, MailError> {
    if repo::origin_queued(conn, mail.origin_id).await? {
        return Ok(false);
    }
    let batch_id = Uuid::now_v7();
    repo::insert_batch(
        conn,
        &repo::NewBatch {
            id: batch_id,
            plugin_id: None,
            kind: mail.kind.as_str(),
            subject: mail.subject.as_str(),
            body: mail.body.as_str(),
            link_path: None,
            accepted: 1,
            skipped: serde_json::json!({}),
        },
    )
    .await?;
    let inserted = repo::insert_account_message(
        conn,
        &repo::NewAccountMessage {
            id: Uuid::now_v7(),
            batch_id,
            user_id: mail.user_id,
            kind: mail.kind.as_str(),
            to_address: mail.address.as_str(),
            to_name: &mail.name,
            origin_id: mail.origin_id,
        },
    )
    .await?;
    if inserted == 0 {
        repo::delete_batch(conn, batch_id).await?;
    }
    Ok(inserted == 1)
}

/// Queues a plugin's mail for its recipients, under the plugin's lock: the hourly count of calls,
/// then for each recipient the opt-out, the daily limit and the dedupe claim, in that order, so a
/// skipped message claims nothing. All in the caller's transaction (ADR-0085).
pub async fn enqueue_plugin(
    conn: &mut PgConnection,
    limits: &Limits,
    mail: PluginMail,
) -> Result<PluginOutcome, MailError> {
    let plugin_id = mail.plugin_id.as_str();
    repo::lock_plugin(conn, plugin_id).await?;
    let (calls, oldest) = repo::calls_in_window(conn, plugin_id, HOUR_SECS).await?;
    if calls >= i64::from(limits.plugin_hourly_calls) {
        return Err(MailError::HourlyLimit {
            retry_after_secs: retry_after(HOUR_SECS, oldest.unwrap_or(0.0)),
        });
    }

    let mut seen = HashSet::new();
    let mut recipients = mail.recipients;
    recipients.retain(|r| seen.insert(r.user_id));

    let mut skipped = Skipped {
        no_address: mail.no_address,
        disabled: mail.disabled,
        ..Skipped::default()
    };

    // 1. The opt-out of the person, per plugin and kind.
    let ids: Vec<Uuid> = recipients.iter().map(|r| r.user_id).collect();
    let opted: HashSet<Uuid> = repo::opted_out(conn, plugin_id, mail.kind.as_str(), &ids)
        .await?
        .into_iter()
        .collect();
    recipients.retain(|r| {
        let keep = !opted.contains(&r.user_id);
        if !keep {
            skipped.opted_out += 1;
        }
        keep
    });

    // 2. The daily limit of the person. A single recipient over it is an error, an audience skips them.
    let ids: Vec<Uuid> = recipients.iter().map(|r| r.user_id).collect();
    let recent: HashMap<Uuid, repo::Recent> = repo::recent_plugin_messages(conn, &ids, DAY_SECS)
        .await?
        .into_iter()
        .map(|r| (r.user_id, r))
        .collect();
    let mut over = None;
    recipients.retain(|r| match recent.get(&r.user_id) {
        Some(recent) if recent.count >= i64::from(limits.recipient_daily) => {
            skipped.rate_limited += 1;
            over = Some(retry_after(DAY_SECS, recent.oldest_age));
            false
        }
        _ => true,
    });
    if mail.single
        && let Some(retry_after_secs) = over
    {
        return Err(MailError::RecipientLimit { retry_after_secs });
    }

    // 3. The dedupe claim, atomic with the rows that follow.
    if let Some(key) = &mail.dedupe_key {
        let ids: Vec<Uuid> = recipients.iter().map(|r| r.user_id).collect();
        let claimed: HashSet<Uuid> =
            repo::claim_dedupe(conn, plugin_id, key.as_str(), &ids, DEDUPE_DAYS)
                .await?
                .into_iter()
                .collect();
        recipients.retain(|r| {
            let keep = claimed.contains(&r.user_id);
            if !keep {
                skipped.duplicate += 1;
            }
            keep
        });
    }

    let batch_id = Uuid::now_v7();
    let accepted = u32::try_from(recipients.len()).unwrap_or(u32::MAX);
    repo::insert_batch(
        conn,
        &repo::NewBatch {
            id: batch_id,
            plugin_id: Some(plugin_id),
            kind: mail.kind.as_str(),
            subject: mail.subject.as_str(),
            body: mail.body.as_str(),
            link_path: mail.link_path.as_ref().map(|p| p.as_str()),
            accepted: i32::try_from(accepted).unwrap_or(i32::MAX),
            skipped: serde_json::json!({
                "opted_out": skipped.opted_out,
                "no_address": skipped.no_address,
                "duplicate": skipped.duplicate,
                "disabled": skipped.disabled,
                "rate_limited": skipped.rate_limited,
            }),
        },
    )
    .await?;
    if !recipients.is_empty() {
        let ids: Vec<Uuid> = recipients.iter().map(|_| Uuid::now_v7()).collect();
        let users: Vec<Uuid> = recipients.iter().map(|r| r.user_id).collect();
        let addresses: Vec<String> = recipients
            .iter()
            .map(|r| r.address.as_str().to_owned())
            .collect();
        let names: Vec<String> = recipients.iter().map(|r| r.name.clone()).collect();
        repo::insert_plugin_messages(
            conn,
            &repo::NewMessages {
                batch_id,
                plugin_id,
                kind: mail.kind.as_str(),
                ids: &ids,
                users: &users,
                addresses: &addresses,
                names: &names,
            },
        )
        .await?;
    }
    Ok(PluginOutcome {
        batch_id,
        accepted,
        skipped,
    })
}

/// Cancels the pending messages of a plugin that was disabled or removed. Enabling it again does
/// not bring them back. Returns how many.
pub async fn cancel_pending(conn: &mut PgConnection, plugin_id: &str) -> Result<u64, MailError> {
    Ok(repo::cancel_pending(conn, plugin_id).await?)
}

/// The key that signs the opt-out links: made on first use.
pub(crate) async fn signing_key(conn: &mut PgConnection) -> Result<Vec<u8>, MailError> {
    if let Some(key) = repo::signing_key(conn).await? {
        return Ok(key);
    }
    let mut key = [0u8; 32];
    rand::rng().fill_bytes(&mut key);
    repo::insert_signing_key(conn, &key).await?;
    // A second process may have won the insert: read what is stored.
    Ok(repo::signing_key(conn).await?.unwrap_or(key.to_vec()))
}

/// The token of the opt-out link of `opt_out`.
pub(crate) fn token(key: &[u8], opt_out: &OptOut) -> Option<String> {
    optout::sign(key, opt_out)
}

/// Applies the opt-out (`enable = false`) or the opt-in (`enable = true`) a token names. `None`
/// for a token that is not ours, with nothing written.
pub(crate) async fn apply_token(
    conn: &mut PgConnection,
    token: &str,
    opt_out: bool,
) -> Result<Option<OptOut>, MailError> {
    let key = match repo::signing_key(conn).await? {
        Some(key) => key,
        // No key was ever made, so no link was ever sent.
        None => return Ok(None),
    };
    let Some(named) = optout::verify(&key, token) else {
        return Ok(None);
    };
    if opt_out {
        repo::set_opt_out(conn, named.user_id, &named.plugin_id, &named.kind).await?;
    } else {
        repo::clear_opt_out(conn, named.user_id, &named.plugin_id, &named.kind).await?;
    }
    Ok(Some(named))
}

pub async fn status(conn: &mut PgConnection) -> Result<Status, MailError> {
    Ok(repo::status(conn).await?)
}

pub async fn list(
    conn: &mut PgConnection,
    filter: &Filter,
    limit: i64,
) -> Result<Vec<JournalEntry>, MailError> {
    Ok(repo::list(conn, filter, limit.clamp(1, 1000)).await?)
}

/// Puts dead messages back in the queue. With `dry_run` it only reports what it would do. A message
/// whose address is already erased cannot be sent and is reported in `erased`, untouched.
pub async fn retry(
    conn: &mut PgConnection,
    scope: &RetryScope,
    dry_run: bool,
) -> Result<Retried, MailError> {
    let (id, plugin) = match scope {
        RetryScope::One(id) => (Some(*id), None),
        RetryScope::Dead { plugin_id } => (None, plugin_id.as_deref()),
    };
    let (live, erased) = repo::dead_in_scope(conn, id, plugin).await?;
    if !dry_run && !live.is_empty() {
        repo::requeue(conn, &live).await?;
    }
    Ok(Retried {
        requeued: live,
        erased,
    })
}

#[cfg(test)]
mod tests;
