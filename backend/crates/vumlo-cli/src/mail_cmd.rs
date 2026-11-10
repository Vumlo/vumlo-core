//! `vumlo-cli mail ...` (ADR-0087): the queue and the delivery journal of the mail layer. Nothing
//! here prints an address: the journal names a person by id. `retry` is the one command that changes
//! the database, so it writes an audit row (no actor, source `cli`) in the same transaction, and
//! writes nothing when it puts nothing back.

use anyhow::{Context, bail};
use chrono::SecondsFormat;
use mail::{Filter, JournalEntry, MessageState, RetryScope};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;
use vumlo_config::{Options, Settings};

use crate::runtime;

/// The most message ids an audit row names; the count is always exact.
const AUDITED_IDS: usize = 200;
/// The longest subject `list` prints.
const SUBJECT_COLUMNS: usize = 50;

async fn ready(settings: &Settings) -> anyhow::Result<PgPool> {
    let pool = runtime::connect(settings).await?;
    db::ensure_migrated_with(&pool, &runtime::migrators())
        .await
        .context("the database is not ready")?;
    Ok(pool)
}

fn time(at: chrono::DateTime<chrono::Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// `mail status`: what waits, what was sent and what failed.
pub async fn status(options: &Options, as_json: bool) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let pool = ready(&loaded.settings).await?;
    let mut conn = pool.acquire().await.context("cannot reach the database")?;
    let status = mail::status(&mut conn).await?;
    if as_json {
        let dead_by: Vec<Value> = status
            .dead_by
            .iter()
            .map(|(plugin, kind, count)| json!({"plugin": plugin, "kind": kind, "dead": count}))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "configured": loaded.settings.mail.is_some(),
                "pending": status.pending,
                "sent_24h": status.sent_24h,
                "dead_24h": status.dead_24h,
                "cancelled_24h": status.cancelled_24h,
                "dead": status.dead_now,
                "oldest_pending": status.oldest_pending.map(time),
                "dead_by": dead_by,
            }))?
        );
        return Ok(());
    }
    if loaded.settings.mail.is_none() {
        println!("[mail] is not configured: nothing is sent, and plugin mail is refused");
    }
    println!("pending        {}", status.pending);
    match status.oldest_pending {
        Some(at) => println!("oldest pending {}", time(at)),
        None => println!("oldest pending -"),
    }
    println!("sent (24 h)    {}", status.sent_24h);
    println!("dead (24 h)    {}", status.dead_24h);
    println!("cancelled (24 h) {}", status.cancelled_24h);
    println!("dead, in all   {}", status.dead_now);
    for (plugin, kind, count) in &status.dead_by {
        println!(
            "  {count:>6} dead  {:<24} {kind}",
            plugin.as_deref().unwrap_or("(account mail)")
        );
    }
    Ok(())
}

pub struct ListArgs {
    pub state: Option<String>,
    pub plugin: Option<String>,
    pub kind: Option<String>,
    pub user: Option<Uuid>,
    pub hours: Option<u32>,
    pub limit: u32,
    pub as_json: bool,
}

fn cut(text: &str) -> String {
    let line = text.lines().next().unwrap_or("");
    if line.chars().count() <= SUBJECT_COLUMNS {
        return line.to_owned();
    }
    let mut short: String = line.chars().take(SUBJECT_COLUMNS - 1).collect();
    short.push('…');
    short
}

fn entry_json(entry: &JournalEntry) -> Value {
    json!({
        "id": entry.id,
        "at": time(entry.created_at),
        "state": entry.state.as_str(),
        "attempts": entry.attempts,
        "plugin": entry.plugin_id,
        "kind": entry.kind,
        "user_id": entry.user_id,
        "subject": entry.subject,
        "error": entry.last_error,
    })
}

/// `mail list`: the newest messages first, by id and person, never by address.
pub async fn list(options: &Options, args: ListArgs) -> anyhow::Result<()> {
    let state = match args.state.as_deref() {
        None => None,
        Some(text) => match text.parse::<MessageState>() {
            Ok(state) => Some(state),
            Err(_) => bail!("--state is one of pending, sent, dead, cancelled"),
        },
    };
    let filter = Filter {
        state,
        plugin_id: args.plugin,
        kind: args.kind,
        user_id: args.user,
        hours: args.hours,
    };
    let loaded = runtime::load(options)?;
    let pool = ready(&loaded.settings).await?;
    let mut conn = pool.acquire().await.context("cannot reach the database")?;
    let entries = mail::list(&mut conn, &filter, i64::from(args.limit)).await?;
    if args.as_json {
        let items: Vec<Value> = entries.iter().map(entry_json).collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }
    if entries.is_empty() {
        println!("no message matches");
        return Ok(());
    }
    for entry in &entries {
        println!(
            "{} {} {:<9} x{} {:<20} {:<16} user {} {}{}",
            entry.id,
            time(entry.created_at),
            entry.state.as_str(),
            entry.attempts,
            entry.plugin_id.as_deref().unwrap_or("(account mail)"),
            entry.kind,
            entry.user_id,
            entry.subject.as_deref().map(cut).unwrap_or_default(),
            entry
                .last_error
                .as_deref()
                .map(|error| format!("  [{error}]"))
                .unwrap_or_default(),
        );
    }
    Ok(())
}

/// `mail retry <id>` and `mail retry --dead [--plugin <id>]`: dead messages go back in the queue.
/// A message whose address is already erased cannot be sent; it is listed and left alone.
pub async fn retry(
    options: &Options,
    id: Option<Uuid>,
    plugin: Option<String>,
    dry_run: bool,
) -> anyhow::Result<()> {
    let scope = match id {
        Some(id) => RetryScope::One(id),
        None => RetryScope::Dead { plugin_id: plugin },
    };
    let loaded = runtime::load(options)?;
    let pool = ready(&loaded.settings).await?;
    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    let retried = mail::retry(&mut tx, &scope, dry_run).await?;
    if !dry_run && let Some(first) = retried.requeued.first() {
        let ids: Vec<&Uuid> = retried.requeued.iter().take(AUDITED_IDS).collect();
        db::insert_cli_audit(
            &mut tx,
            "mail.retry",
            "mail_message",
            *first,
            json!({
                "count": retried.requeued.len(),
                "ids": ids,
                "plugin": match &scope {
                    RetryScope::Dead { plugin_id } => plugin_id.clone(),
                    RetryScope::One(_) => None,
                },
            }),
        )
        .await?;
    }
    tx.commit().await.context("cannot commit")?;

    let verb = if dry_run {
        "would put back"
    } else {
        "put back"
    };
    println!("{verb} {} message(s) in the queue", retried.requeued.len());
    if !retried.erased.is_empty() {
        println!(
            "{} message(s) cannot be sent again, their address was erased: {}",
            retried.erased.len(),
            retried
                .erased
                .iter()
                .map(Uuid::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if retried.requeued.is_empty() && retried.erased.is_empty() {
        println!("nothing was changed");
    }
    Ok(())
}
