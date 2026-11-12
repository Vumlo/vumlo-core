//! Student work: drafts, submitted attempts and deadlines (ADR-0011, ADR-0035).

mod model;
mod repo;
mod routes;
mod service;

pub use model::{
    Attempt, AutoTotal, Draft, DueDraft, FormFacts, LatestAttempt, LatestContent, MySubmission,
    QuizResult, Responses, StudentName, StudentSubmission, Window,
};
pub use routes::router;
pub use service::{
    AssessmentError, authorize_submit, authorize_view_submission, auto_submit, auto_total,
    discard_draft, due_drafts, form_facts, has_submitted_for_assignment, has_submitted_in_course,
    has_submitted_in_module, list_submissions, may_read_submission_files, my_submission,
    purge_assignment, purge_course, purge_module, quiz_result, responses, save_draft,
    skip_auto_submit, student_attempts, students_with_attempts, submit,
};

// Seeds the references of a submission for the tests of `learning::files`.
#[cfg(test)]
pub use repo::replace_submission_files;
