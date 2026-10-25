use chrono::NaiveDate;
use sqlx::PgConnection;

use super::model::TermUsage;
use crate::access::Actor;
use crate::access::{self, Action};
use crate::learning::LearningError;
use crate::learning::teaching;
use crate::org;

/// How many regular courses each term has, and whether it is current or over on `today`
/// (ADR-0053). Admins only. The terms come from `org` and the counts from `teaching`.
pub async fn term_stats(
    conn: &mut PgConnection,
    actor: &Actor,
    today: NaiveDate,
) -> Result<Vec<TermUsage>, LearningError> {
    access::authorize(actor, Action::ViewAdminReports)?;
    let terms = org::terms_newest_first(conn, actor).await?;
    let counts = teaching::regular_courses_per_term(conn).await?;
    Ok(terms
        .into_iter()
        .map(|term| TermUsage {
            term_id: term.id,
            courses: counts.get(&term.id).copied().unwrap_or(0),
            current: (term.starts_on..=term.ends_on).contains(&today),
            ended: term.ends_on < today,
        })
        .collect())
}
