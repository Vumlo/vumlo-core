use access::Actor;
use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{LearningError, items, submissions};
use activity::EffectiveProfiles;
use assessment::{AssessmentError, Attempt, Draft, DueDraft};
use content::{Assignment, ContentTitle, Item, ItemRules, NewItemInput};

/// `db_test` for the unit tests of this crate: what they emit needs the registry of subscribers
/// installed, as the server does at start (ADR-0093).
pub(crate) async fn db_test(body: impl AsyncFnOnce(PgPool)) {
    events::testing::ensure_installed(crate::subscribers);
    test_support::db_test(body).await;
}

/// The error of `assessment` in the error of a flow; any other error is a fault of the test.
fn assessment_error(error: LearningError) -> AssessmentError {
    match error {
        LearningError::Assessment(error) => error,
        other => panic!("a flow failed outside assessment: {other:?}"),
    }
}

/// Saves a draft the way the route does: the files it names are checked in the flow.
pub(crate) async fn save_draft(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    content: &serde_json::Value,
    profiles: &EffectiveProfiles,
    now: DateTime<Utc>,
) -> Result<Draft, AssessmentError> {
    submissions::save_draft(conn, actor, assignment_id, content, profiles, now)
        .await
        .map_err(assessment_error)
}

/// Submits the draft the way the route does.
pub(crate) async fn submit(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    profiles: &EffectiveProfiles,
    now: DateTime<Utc>,
) -> Result<Attempt, AssessmentError> {
    submissions::submit(conn, actor, assignment_id, profiles, now)
        .await
        .map_err(assessment_error)
}

/// Submits a due draft the way the scheduler does.
pub(crate) async fn auto_submit(
    conn: &mut PgConnection,
    due: &DueDraft,
    profiles: &EffectiveProfiles,
    now: DateTime<Utc>,
) -> Result<Attempt, AssessmentError> {
    submissions::auto_submit(conn, due, profiles, now)
        .await
        .map_err(assessment_error)
}

/// Creates a `text` item, or an assignment when `assignment` is given, the way the route does: the
/// files it names are checked in the flow.
pub(crate) async fn create_item(
    conn: &mut PgConnection,
    actor: &Actor,
    rules: ItemRules<'_>,
    module_id: Uuid,
    title: ContentTitle,
    body: Option<serde_json::Value>,
    assignment: Option<Assignment>,
) -> Result<Item, LearningError> {
    let input = NewItemInput::document(title, body, assignment);
    items::create_item(conn, actor, rules, module_id, input).await
}
