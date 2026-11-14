use sqlx::PgPool;

/// `db_test` for the unit tests of this crate: what they emit needs a registry of subscribers
/// installed, as the server does at start (ADR-0093). `content` has no subscribers of its own, so
/// the registry is empty; the flows that subscribe are tested in `learning`. What they read needs
/// the activity types (ADR-0097).
pub(crate) async fn db_test(body: impl AsyncFnOnce(PgPool)) {
    events::testing::ensure_installed(events::Subscribers::new);
    install_types();
    test_support::db_test(body).await;
}

pub(crate) fn install_types() {
    activity::ensure_installed(|| crate::builtin_types().unwrap());
}

/// The built-in type `id`, from the installed registry.
pub(crate) fn kind(id: &str) -> activity::Kind {
    install_types();
    activity::get(id).unwrap()
}
