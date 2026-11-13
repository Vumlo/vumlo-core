use sqlx::PgPool;

/// `db_test` for the unit tests of this crate: what they emit needs a registry of subscribers
/// installed, as the server does at start (ADR-0093). `content` has no subscribers of its own, so
/// the registry is empty; the flows that subscribe are tested in `learning`.
pub(crate) async fn db_test(body: impl AsyncFnOnce(PgPool)) {
    events::testing::ensure_installed(events::Subscribers::new);
    test_support::db_test(body).await;
}
