//! Student work: drafts, submitted attempts and deadlines (ADR-0011, ADR-0035).

mod model;
mod repo;
mod routes;
mod service;

pub use model::{
    Attempt, Draft, FormFacts, LatestAttempt, LatestContent, MySubmission, PluginSubmission,
    QuizResult, Responses, StudentName, StudentSubmission, Window,
};
pub use routes::router;
pub use service::{
    AssessmentError, authorize_submit, authorize_view_submission, discard_draft, ensure_reportable,
    form_facts, has_submitted_for_assignment, has_submitted_in_course, has_submitted_in_module,
    list_submissions, may_read_submission_files, my_submission, purge_assignment, purge_course,
    purge_module, quiz_result, responses, save_draft, student_attempts, submission_for_plugin,
    submit,
};

// Seeds the references of a submission for the tests of `learning::files`.
#[cfg(test)]
pub use repo::replace_submission_files;
