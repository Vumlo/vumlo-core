//! Sending the events a plugin is owed (ADR-0066). The worker claims due deliveries the way the
//! outbox worker does (ADR-0045): in a short transaction, with a lease, then sends with no
//! transaction open. Each request is signed with the plugin's derived secret and carries the time,
//! so a plugin can refuse a replay.

use std::time::Duration;

use chrono::Utc;
use events::{MAX_ATTEMPTS, backoff};
use module_api::OutboundClient;
use serde_json::json;
use sqlx::PgPool;

use crate::PluginsError;
use crate::repo::{self, Claimed};
use crate::secret::{SecretKey, derive_secret, sign};

const BATCH: i64 = 20;
/// A delivery whose worker died comes back after this.
const LEASE: Duration = Duration::from_secs(5 * 60);
const POLL: Duration = Duration::from_secs(2);
const PRUNE_EVERY: Duration = Duration::from_secs(60 * 60);
const CONNECT: Duration = Duration::from_secs(3);
/// A plugin must answer 2xx quickly and work afterwards.
const TOTAL: Duration = Duration::from_secs(5);
const MAX_RESPONSE_BYTES: usize = 4 * 1024;
const MAX_ERROR_CHARS: usize = 500;

/// Sends the events plugins are owed.
pub struct Deliverer {
    key: SecretKey,
    client: OutboundClient,
    /// Days the log of plugin API calls is kept (ADR-0079).
    log_days: u32,
}

impl Deliverer {
    pub fn new(key: SecretKey, log_days: u32) -> Result<Self, PluginsError> {
        let client = OutboundClient::new(CONNECT, TOTAL, MAX_RESPONSE_BYTES, 0)
            .map_err(|e| PluginsError::Fetch(e.to_string()))?;
        Ok(Self {
            key,
            client,
            log_days,
        })
    }

    /// Deletes the log lines older than the retention. There is no scheduler yet (ADR-0007), so the
    /// worker that already wakes every few seconds does it, about once an hour.
    pub async fn prune(&self, pool: &PgPool) -> Result<u64, PluginsError> {
        let mut conn = pool.acquire().await?;
        let days = i32::try_from(self.log_days).unwrap_or(i32::MAX);
        Ok(repo::prune_calls(&mut conn, days).await?)
    }

    /// Claims one batch and sends each delivery. A 2xx answer is delivered; anything else, and a
    /// plugin that does not answer, is a failure that is retried after 10 s, 20 s, 40 s, ... and
    /// after the fifth is a dead letter. Returns how many deliveries it claimed.
    pub async fn once(&self, pool: &PgPool) -> Result<usize, PluginsError> {
        let claimed = {
            let mut conn = pool.acquire().await?;
            repo::claim(&mut conn, BATCH, LEASE.as_secs_f64()).await?
        };
        let count = claimed.len();
        // ponytail: one delivery at a time, so a plugin that takes its five seconds holds up the
        // ones behind it; send a batch concurrently when plugins are many or slow.
        for delivery in claimed {
            let outcome = self.send(&delivery).await;
            let mut conn = pool.acquire().await?;
            match outcome {
                Ok(()) => repo::mark_delivered(&mut conn, delivery.id).await?,
                Err(error) => {
                    let give_up = delivery.attempts >= MAX_ATTEMPTS;
                    let text: String = error.chars().take(MAX_ERROR_CHARS).collect();
                    if give_up {
                        tracing::error!(delivery = %delivery.id, install = %delivery.install_id, kind = %delivery.event_type, attempts = delivery.attempts, error = %text, "plugin delivery given up");
                    } else {
                        tracing::warn!(delivery = %delivery.id, install = %delivery.install_id, kind = %delivery.event_type, attempts = delivery.attempts, error = %text, "plugin delivery failed, will retry");
                    }
                    repo::mark_failed(
                        &mut conn,
                        delivery.id,
                        &text,
                        give_up,
                        backoff(delivery.attempts).as_secs_f64(),
                    )
                    .await?;
                }
            }
        }
        Ok(count)
    }

    /// The error text goes to `last_error`: it holds a status or a transport failure, never the
    /// secret, the signature or the URL.
    async fn send(&self, delivery: &Claimed) -> Result<(), String> {
        let secret = derive_secret(&self.key, delivery.install_id, delivery.secret_version)
            .map_err(|e| e.to_string())?;
        let body = serde_json::to_vec(&json!({
            "id": delivery.event_id,
            "type": delivery.event_type,
            "occurred_at": delivery.occurred_at,
            "data": delivery.data,
        }))
        .map_err(|e| e.to_string())?;
        let timestamp = Utc::now().timestamp();
        let signature = sign(&secret, timestamp, &body).map_err(|e| e.to_string())?;
        let timestamp = timestamp.to_string();
        let (status, _) = self
            .client
            .post(
                &format!("{}/events", delivery.base_url),
                &[
                    ("content-type", "application/json"),
                    ("x-vumlo-timestamp", &timestamp),
                    ("x-vumlo-signature", &signature),
                ],
                body,
            )
            .await
            .map_err(|e| e.to_string())?;
        if (200..300).contains(&status) {
            Ok(())
        } else {
            Err(format!("the plugin answered HTTP {status}"))
        }
    }

    /// Delivers until the process ends.
    pub async fn run(self, pool: PgPool) {
        let mut last_prune: Option<std::time::Instant> = None;
        loop {
            if last_prune.is_none_or(|at| at.elapsed() >= PRUNE_EVERY) {
                last_prune = Some(std::time::Instant::now());
                match self.prune(&pool).await {
                    Ok(0) => {}
                    Ok(n) => tracing::info!(lines = n, "old plugin API calls were deleted"),
                    Err(error) => tracing::error!(%error, "the plugin call log was not pruned"),
                }
            }
            match self.once(&pool).await {
                Ok(n) if n as i64 >= BATCH => continue,
                Ok(_) => {}
                Err(error) => tracing::error!(%error, "the plugin delivery worker failed a round"),
            }
            tokio::time::sleep(POLL).await;
        }
    }
}

#[cfg(test)]
mod tests;
