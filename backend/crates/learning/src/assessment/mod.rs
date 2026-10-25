//! Student work: drafts, submitted attempts and deadlines (ADR-0011, ADR-0035).

mod model;
mod repo;
mod routes;
mod service;

pub use model::{
    Attempt, Draft, FormFacts, LatestAttempt, LatestContent, MySubmission, QuizResult, Responses,
    StudentName, StudentSubmission, Window,
};
pub use routes::router;
pub use service::{
    AssessmentError, discard_draft, form_facts, has_submitted_for_assignment,
    has_submitted_in_course, has_submitted_in_module, list_submissions, my_submission,
    purge_assignment, purge_course, purge_module, quiz_result, responses, save_draft,
    student_attempts, submit,
};
