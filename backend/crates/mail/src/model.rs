//! The types of the mail layer. Each text a plugin writes is a newtype that checks itself, so a
//! value that exists is a value that is fit to be stored and sent (ADR-0085).

use std::fmt;

use chrono::{DateTime, Utc};
use common::text_enum;
use uuid::Uuid;

/// A text that does not fit. `field` names the part of the request to blame.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ModelError {
    pub field: &'static str,
    pub message: &'static str,
}

fn invalid(field: &'static str, message: &'static str) -> ModelError {
    ModelError { field, message }
}

macro_rules! text_newtype {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, PartialEq, Eq)]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                // The text of a mail and an address are personal data: never in a log line.
                write!(f, "{}(..)", stringify!($name))
            }
        }
    };
}

text_newtype!(
    /// A slug of at most 40 characters that groups mails for opt-out and statistics.
    Kind
);
text_newtype!(
    /// One line, 1 to 200 characters, no control character.
    Subject
);
text_newtype!(
    /// Plain text, 1 to 20000 characters; line feed, carriage return and tab are the only controls.
    Body
);
text_newtype!(
    /// A path into the web app: one leading `/`, at most 500 characters.
    LinkPath
);
text_newtype!(
    /// Up to 100 printable characters.
    DedupeKey
);
text_newtype!(
    /// A mailbox address that lettre accepts.
    Address
);

impl Kind {
    pub fn parse(text: &str) -> Result<Self, ModelError> {
        let mut chars = text.chars();
        let ok = chars.next().is_some_and(|c| c.is_ascii_lowercase())
            && text.len() <= 40
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
        if ok {
            Ok(Self(text.to_owned()))
        } else {
            Err(invalid(
                "kind",
                "must be a slug of at most 40 characters: a lowercase letter, then lowercase letters, digits, - or _",
            ))
        }
    }
}

impl Subject {
    pub fn parse(text: &str) -> Result<Self, ModelError> {
        let length = text.chars().count();
        if !(1..=200).contains(&length) || text.chars().any(char::is_control) {
            return Err(invalid(
                "subject",
                "must be 1 to 200 characters with no control character",
            ));
        }
        Ok(Self(text.to_owned()))
    }
}

impl Body {
    pub fn parse(text: &str) -> Result<Self, ModelError> {
        let length = text.chars().count();
        let controls = text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'));
        if !(1..=20_000).contains(&length) || controls {
            return Err(invalid(
                "text",
                "must be 1 to 20000 characters of plain text with no control character but line breaks and tabs",
            ));
        }
        Ok(Self(text.to_owned()))
    }
}

impl LinkPath {
    pub fn parse(text: &str) -> Result<Self, ModelError> {
        let ok = text.len() <= 500
            && text.starts_with('/')
            && !text.contains("//")
            && !text.contains('\\')
            && !text.chars().any(|c| c.is_whitespace() || c.is_control());
        if ok {
            Ok(Self(text.to_owned()))
        } else {
            Err(invalid(
                "link_path",
                "must be a path of at most 500 characters: one leading /, no //, no backslash, no whitespace",
            ))
        }
    }
}

impl DedupeKey {
    pub fn parse(text: &str) -> Result<Self, ModelError> {
        let length = text.chars().count();
        if !(1..=100).contains(&length) || text.chars().any(char::is_control) {
            return Err(invalid(
                "dedupe_key",
                "must be 1 to 100 printable characters",
            ));
        }
        Ok(Self(text.to_owned()))
    }
}

impl Address {
    pub fn parse(text: &str) -> Result<Self, ModelError> {
        if text.len() > 254 || text.parse::<lettre::Address>().is_err() {
            return Err(invalid("address", "is not a mailbox address"));
        }
        Ok(Self(text.to_owned()))
    }
}

/// Who gets a mail. The layer cannot read `auth`, so the caller passes the address, and the layer
/// copies it into the row.
#[derive(Debug, Clone)]
pub struct Recipient {
    pub user_id: Uuid,
    pub name: String,
    pub address: Address,
}

/// A mail the core sends about an account (verification, password reset). Never opt-out-able and
/// never subject to a plugin limit.
#[derive(Debug, Clone)]
pub struct AccountMail {
    /// The outbox event this answers: a second call with the same id queues nothing.
    pub origin_id: Uuid,
    pub kind: Kind,
    pub user_id: Uuid,
    pub name: String,
    pub address: Address,
    pub subject: Subject,
    pub body: Body,
}

/// One call of a plugin to send mail (ADR-0085).
#[derive(Debug, Clone)]
pub struct PluginMail {
    pub plugin_id: String,
    pub kind: Kind,
    pub subject: Subject,
    pub body: Body,
    pub link_path: Option<LinkPath>,
    pub dedupe_key: Option<DedupeKey>,
    pub recipients: Vec<Recipient>,
    /// Addressed to one `user_id`: over the daily limit is `429`. For an audience that person is
    /// skipped and the rest go.
    pub single: bool,
    /// People the caller left out before this call, for the numbers: no verified address, and a
    /// disabled account.
    pub no_address: u32,
    pub disabled: u32,
}

/// The limits that apply to plugin mail, from `[mail]`.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub plugin_hourly_calls: u32,
    pub recipient_daily: u32,
}

impl From<&vumlo_config::MailSettings> for Limits {
    fn from(settings: &vumlo_config::MailSettings) -> Self {
        Self {
            plugin_hourly_calls: settings.plugin_hourly_calls,
            recipient_daily: settings.recipient_daily,
        }
    }
}

/// How many people a call did not mail, by reason. All five are always present in the answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Skipped {
    pub opted_out: u32,
    pub no_address: u32,
    pub duplicate: u32,
    pub disabled: u32,
    pub rate_limited: u32,
}

impl Skipped {
    pub fn total(&self) -> u32 {
        self.opted_out + self.no_address + self.duplicate + self.disabled + self.rate_limited
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PluginOutcome {
    pub batch_id: Uuid,
    pub accepted: u32,
    pub skipped: Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageState {
    Pending,
    Sent,
    Dead,
    Cancelled,
}

text_enum!(MessageState {
    Pending => "pending",
    Sent => "sent",
    Dead => "dead",
    Cancelled => "cancelled",
});

/// One row of the journal, as the administrator sees it. No address.
#[derive(Debug, Clone)]
pub struct JournalEntry {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub state: MessageState,
    pub attempts: i32,
    pub plugin_id: Option<String>,
    pub kind: String,
    pub user_id: Uuid,
    /// Kept while the batch is; cut by the caller for display.
    pub subject: Option<String>,
    pub last_error: Option<String>,
}

/// What `mail list` asks for.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub state: Option<MessageState>,
    pub plugin_id: Option<String>,
    pub kind: Option<String>,
    pub user_id: Option<Uuid>,
    pub hours: Option<u32>,
}

/// What `mail retry` puts back.
#[derive(Debug, Clone)]
pub enum RetryScope {
    One(Uuid),
    Dead { plugin_id: Option<String> },
}

/// The result of a retry: ids put back and ids left (their address is already erased).
#[derive(Debug, Clone, Default)]
pub struct Retried {
    pub requeued: Vec<Uuid>,
    pub erased: Vec<Uuid>,
}

/// The counts of `mail status`.
#[derive(Debug, Clone, Default)]
pub struct Status {
    pub pending: i64,
    pub sent_24h: i64,
    pub dead_24h: i64,
    pub cancelled_24h: i64,
    pub dead_now: i64,
    pub oldest_pending: Option<DateTime<Utc>>,
    /// Dead messages by (plugin, kind).
    pub dead_by: Vec<(Option<String>, String, i64)>,
}

#[cfg(test)]
mod tests;
