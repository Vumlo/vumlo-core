//! The mail queue and journal in the console (ADR-0095). Nothing here carries an address: a person
//! is an id.

use chrono::{DateTime, Utc};
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// What waits, what was sent and what died.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct MailStatus {
    /// Whether the installation has `[mail]`; without it nothing is sent.
    pub configured: bool,
    /// `smtp` or `file`, when configured.
    pub transport: Option<String>,
    pub pending: i64,
    pub sent_24h: i64,
    pub dead_24h: i64,
    pub cancelled_24h: i64,
    /// Dead messages in the journal now.
    pub dead_now: i64,
    pub oldest_pending: Option<DateTime<Utc>>,
    pub dead_by: Vec<MailDeadBy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct MailDeadBy {
    /// `None` for the mail of the core itself (accounts).
    pub plugin_id: Option<String>,
    pub kind: String,
    pub dead: i64,
}

/// One message of the journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct MailMessage {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    /// `pending`, `sent`, `dead` or `cancelled`.
    pub state: String,
    pub attempts: i32,
    pub plugin_id: Option<String>,
    pub kind: String,
    /// The person it is for.
    pub user_id: Uuid,
    /// The subject, cut for display; kept only while the batch is.
    pub subject: Option<String>,
    /// A class such as `smtp 451`, never the server's text.
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct MailMessages {
    pub items: Vec<MailMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct MailMessagesQuery {
    /// `pending`, `sent`, `dead` or `cancelled`.
    #[garde(length(max = 16))]
    pub state: Option<String>,
    #[garde(length(max = 64))]
    pub plugin_id: Option<String>,
    #[garde(length(max = 40))]
    pub kind: Option<String>,
    #[garde(skip)]
    pub user_id: Option<Uuid>,
    /// Only messages of the last this many hours, 1 to 8760.
    #[garde(range(min = 1, max = 8760))]
    pub hours: Option<u32>,
    /// The newest this many, 1 to 200; default 50.
    #[garde(range(min = 1, max = 200))]
    pub limit: Option<i64>,
}

/// What to put back: one message by `id`, or, without it, the dead mail of `plugin_id` (all dead
/// mail without that either).
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct RetryMailRequest {
    #[garde(skip)]
    pub id: Option<Uuid>,
    #[garde(length(max = 64))]
    pub plugin_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RetryMailResponse {
    /// Put back in the queue.
    pub requeued: i64,
    /// Left dead: their address is already erased.
    pub erased: i64,
}
