//! Course announcements (ADR-0076).

use chrono::{DateTime, Utc};
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

pub const ANNOUNCEMENT_TITLE_MAX: usize = 200;
pub const ANNOUNCEMENT_BODY_MAX: usize = 10_000;

/// Something a teacher told the course. Plain text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Announcement {
    pub id: Uuid,
    pub course_id: Uuid,
    pub author_id: Uuid,
    pub title: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A title is words, not blank space, and a body says something.
fn not_blank(value: &str, _: &()) -> garde::Result {
    if value.trim().is_empty() {
        Err(garde::Error::new("must not be blank"))
    } else {
        Ok(())
    }
}

/// Publishes an announcement to the course at once.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateAnnouncementRequest {
    /// 1 to 200 characters after trimming.
    #[garde(length(chars, min = 1, max = ANNOUNCEMENT_TITLE_MAX), custom(not_blank))]
    pub title: String,
    /// 1 to 10 000 characters of plain text.
    #[garde(length(chars, min = 1, max = ANNOUNCEMENT_BODY_MAX), custom(not_blank))]
    pub body: String,
}

/// Changes an announcement. A field that is absent stays. Nothing is sent to the plugins.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateAnnouncementRequest {
    #[garde(inner(length(chars, min = 1, max = ANNOUNCEMENT_TITLE_MAX), custom(not_blank)))]
    pub title: Option<String>,
    #[garde(inner(length(chars, min = 1, max = ANNOUNCEMENT_BODY_MAX), custom(not_blank)))]
    pub body: Option<String>,
}
