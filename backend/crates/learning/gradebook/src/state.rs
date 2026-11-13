use sqlx::PgPool;
use vumlo_config::GradingScale;

/// What the handlers of `gradebook` need from the application.
#[derive(Clone)]
pub struct GradebookState {
    pub db: PgPool,
    /// `institution.grading_scale`: whether the gradebook shows totals (ADR-0022, ADR-0089).
    pub grading_scale: GradingScale,
}
