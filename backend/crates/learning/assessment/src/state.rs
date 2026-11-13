use sqlx::PgPool;

/// What the handlers of `assessment` need from the application.
#[derive(Clone)]
pub struct AssessmentState {
    pub db: PgPool,
}
