use sqlx::PgConnection;

use super::model::{AuditFilter, AuditRow};
use super::repo;
use crate::learning::LearningError;
use access::Actor;
use access::{self, Action};

/// The rows of the audit log, newest first, paged by id (ADR-0053). Admins only.
pub async fn list_audit(
    conn: &mut PgConnection,
    actor: &Actor,
    mut filter: AuditFilter,
    before: Option<uuid::Uuid>,
    limit: i64,
) -> Result<Vec<AuditRow>, LearningError> {
    access::authorize(actor, Action::ViewAdminReports)?;
    let pattern = filter.text.take().map(|text| like_pattern(text.trim()));
    Ok(repo::list_audit(conn, &filter, pattern.as_deref(), before, limit).await?)
}

/// `text` as a `LIKE` pattern that matches it as it is: `%`, `_` and `\` are escaped.
fn like_pattern(text: &str) -> String {
    let mut pattern = String::with_capacity(text.len() + 2);
    pattern.push('%');
    for c in text.chars() {
        if matches!(c, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    pattern
}
