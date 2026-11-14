//! The five types built into the server, as an activity registry (ADR-0097).

use activity::{InstallError, Types};

use super::file::File;
use super::form::Form;
use super::no_submission::NoSubmission;
use super::quiz::Quiz;
use super::text::Text;

static TEXT: Text = Text;
static NO_SUBMISSION: NoSubmission = NoSubmission;
static QUIZ: Quiz = Quiz;
static FORM: Form = Form;
static FILE: File = File;

/// The built-in types. The composition root adds the types of modules and installs the result
/// once; it handles the error at startup.
pub fn builtin_types() -> Result<Types, InstallError> {
    Types::new()
        .with(&TEXT)?
        .with(&NO_SUBMISSION)?
        .with(&QUIZ)?
        .with(&FORM)?
        .with(&FILE)
}

#[cfg(test)]
mod tests;
