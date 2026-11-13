//! What an administrator does to the mail queue from the console (ADR-0095): the actor-checked,
//! audited side of the functions `vumlo-cli mail ...` calls. Nothing here returns an address.

use access::{Action, Actor};
use events::AuditEntry;
use serde_json::json;
use sqlx::PgConnection;

use crate::error::MailError;
use crate::model::{Filter, JournalEntry, Retried, RetryScope, Status};
use crate::service;

/// The most message ids an audit entry names; the count is always exact.
const AUDITED_IDS: usize = 200;

pub async fn admin_status(conn: &mut PgConnection, actor: &Actor) -> Result<Status, MailError> {
    access::authorize(actor, Action::ViewMail)?;
    service::status(conn).await
}

pub async fn admin_list(
    conn: &mut PgConnection,
    actor: &Actor,
    filter: &Filter,
    limit: i64,
) -> Result<Vec<JournalEntry>, MailError> {
    access::authorize(actor, Action::ViewMail)?;
    service::list(conn, filter, limit).await
}

/// Puts dead messages back in the queue. Something put back is audited with the count and the ids
/// (at most 200); nothing put back writes nothing.
pub async fn admin_retry(
    conn: &mut PgConnection,
    actor: &Actor,
    scope: &RetryScope,
) -> Result<Retried, MailError> {
    access::authorize(actor, Action::ManageMail)?;
    let retried = service::retry(conn, scope, false).await?;
    if let Some(first) = retried.requeued.first() {
        let ids: Vec<_> = retried.requeued.iter().take(AUDITED_IDS).collect();
        events::audit(
            conn,
            actor,
            AuditEntry {
                action: "mail.retry",
                resource_type: "mail_message",
                resource_id: *first,
                meta: json!({
                    "count": retried.requeued.len(),
                    "ids": ids,
                    "plugin": match scope {
                        RetryScope::Dead { plugin_id } => plugin_id.clone(),
                        RetryScope::One(_) => None,
                    },
                }),
            },
        )
        .await?;
    }
    Ok(retried)
}
