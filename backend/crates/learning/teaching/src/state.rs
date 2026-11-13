use sqlx::PgPool;

/// What the handlers of `teaching` need from the application.
#[derive(Clone)]
pub struct TeachingState {
    pub db: PgPool,
}
