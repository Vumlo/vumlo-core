use access::Actor;
use sqlx::PgConnection;
use sqlx::PgPool;
use uuid::Uuid;

use crate::content::{Assignment, ContentTitle, Item, ItemRules, NewItemInput};
use crate::{LearningError, items};

/// `db_test` for the unit tests of this crate: what they emit needs the registry of subscribers
/// installed, as the server does at start (ADR-0093).
pub(crate) async fn db_test(body: impl AsyncFnOnce(PgPool)) {
    events::testing::ensure_installed(crate::subscribers);
    test_support::db_test(body).await;
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
