//! The Buddy's domain types.

use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
}
common::text_enum!(ChatRole { User => "user", Assistant => "assistant" });

/// One line of the visible chat.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub id: Uuid,
    pub role: ChatRole,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

/// What the client tells the Buddy about where the user is. Hints only: every tool call is
/// authorized again by the service function it reaches.
#[derive(Debug, Clone, Default)]
pub struct PageContext {
    pub page: Option<String>,
    pub course_id: Option<Uuid>,
    pub item_id: Option<Uuid>,
}

/// A change the Buddy has prepared and the user may confirm. Valid for a day.
#[derive(Debug, Clone)]
pub struct Proposal {
    pub id: Uuid,
    pub tool: String,
    /// What the confirm runs: JSON, in the shape of `tools::Write`.
    pub args: String,
    /// What the card shows: a JSON object of strings.
    pub facts: String,
    pub created_at: DateTime<Utc>,
}
