//! Grades, publishing and the gradebook read models (ADR-0009, ADR-0015, ADR-0028).

mod model;
mod repo;
mod routes;
mod service;

pub use model::{
    Cell, Column, ColumnStatus, Feedback, FeedbackInput, FieldProblem, GradeInput, GradebookPage,
    GradebookRow, HistoryActor, HistoryEntry, ModelError, MyGrade, PublishedGrade, Score, Students,
    TeachingOverview,
};
pub use routes::router;
pub use service::{
    GradebookError, grade_history, gradebook, has_grades_for_assignment, has_grades_in_course,
    has_grades_in_module, max_score, my_grades, publish_grades, published_grade, purge_assignment,
    purge_course, purge_module, save_auto_grade, save_grade, teaching_overview,
};
