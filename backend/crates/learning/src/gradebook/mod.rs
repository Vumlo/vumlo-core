//! Grades, publishing and the gradebook read models (ADR-0009, ADR-0015, ADR-0028).

mod export;
mod layout;
mod model;
mod repo;
mod routes;
mod service;

pub use model::{
    Cell, Column, ColumnStatus, Feedback, FeedbackInput, FieldProblem, GradeInput, GradebookPage,
    GradebookRow, HistoryActor, HistoryEntry, ItemMeta, ModelError, MyGrade, MyWork, Progress,
    PublishedGrade, Score, StaffFacts, Students, TeachingOverview, TodoCounts, WorkEntry,
    WorkFacts, WorkStatus,
};
pub use routes::router;
pub use service::{
    GradebookError, copy_layout, grade_history, gradebook, has_grades_for_assignment,
    has_grades_in_course, has_grades_in_module, item_meta, max_score, my_grades, my_work,
    publish_grades, published_grade, purge_assignment, purge_course, purge_module, save_auto_grade,
    save_grade, student_next, student_progress, teaching_overview, todo_counts,
};
