use sqlx::PgPool;

/// `db_test` for the unit tests of this crate: what they emit needs the registry of subscribers
/// installed, as the server does at start (ADR-0093).
pub(crate) async fn db_test(body: impl AsyncFnOnce(PgPool)) {
    events::testing::ensure_installed(crate::subscribers);
    test_support::db_test(body).await;
}
