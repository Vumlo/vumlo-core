use sqlx::PgPool;

/// `db_test` for the unit tests of this crate: what they emit needs a registry of subscribers
/// installed, as the server does at start (ADR-0093). `org` has no subscribers of its own.
pub(crate) async fn db_test(body: impl AsyncFnOnce(PgPool)) {
    events::testing::ensure_installed(events::Subscribers::new);
    test_support::db_test(body).await;
}
