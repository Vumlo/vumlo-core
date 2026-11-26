//! Student work: drafts, submitted attempts and deadlines (ADR-0011, ADR-0035).

mod model;
mod repo;
mod routes;
mod service;
mod state;

pub use model::{
    Attempt, AttemptResult, AutoTotal, Draft, DueDraft, LatestAttempt, LatestContent, MySubmission,
    PublishedScore, Responses, ResultStart, StudentName, StudentSubmission, Window,
};
pub use routes::{api_attempt, api_draft, router};
pub use service::{
    AssessmentError, DraftSave, SubmitStart, authorize_submit, authorize_view_submission,
    auto_total, begin_auto_submit, begin_result, begin_save_draft, begin_submit, class_facts,
    discard_draft, due_drafts, finish_result, finish_save_draft, finish_submit,
    has_submitted_for_assignment, has_submitted_in_course, has_submitted_in_module,
    list_submissions, may_read_submission_files, my_submission, purge_assignment, purge_course,
    purge_module, responses, skip_auto_submit, student_attempts, students_with_attempts,
};
pub use state::AssessmentState;

// Seeds the references of a submission for the tests of `learning::files`.
#[cfg(feature = "testing")]
pub use repo::replace_submission_files;
