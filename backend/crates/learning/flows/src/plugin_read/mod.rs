//! What a plugin reads about the courses it may reach (ADR-0066, ADR-0074). A flow of `learning`:
//! it looks at content, student work, grades and the roster together, and a plugin has no `Actor`,
//! so every function here takes the [`PluginCaller`](access::PluginCaller), checks its scope and
//! its course first, and shows only what a student could be shown: submitted attempts, published
//! grades, assignments that are open, and no answer key.

mod model;
mod repo;
mod service;

pub use model::{
    Cursor, MailAudience, MailPerson, MailRecipients, MailTarget, PluginAnnouncement,
    PluginAssignment, PluginAttempt, PluginContact, PluginDeletion, PluginFormAnswers, PluginGrade,
    PluginList, PluginMember, PluginPage, PluginQuiz, PluginSubmission, PluginSubmissionRow,
};
pub use service::{
    Deleted, PluginReadError, announcements, assignment, assignments, attempts, contacts, deleted,
    form_answers, grades, mail_recipients, members, quiz, reportable_assignment, reportable_course,
    reportable_submission, submission, submissions,
};

#[cfg(test)]
mod tests;
