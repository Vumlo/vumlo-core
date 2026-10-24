use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::auth::User;
use crate::learning::teaching::UserCourseFacts;

/// What one person does in the institution (ADR-0053).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserFacts {
    pub user: User,
    pub last_login_at: Option<DateTime<Utc>>,
    pub groups: Vec<(Uuid, String)>,
    pub courses: UserCourseFacts,
}
