//! The seven types built into the server, as an activity registry (ADR-0097, ADR-0098): the five
//! assignment types and the two that have no points, a lecture and a shared file.

use crate::{InstallError, Types};

use super::file::File;
use super::form::Form;
use super::lecture::Lecture;
use super::no_submission::NoSubmission;
use super::quiz::Quiz;
use super::shared_file::SharedFile;
use super::text::Text;

static TEXT: Text = Text;
static NO_SUBMISSION: NoSubmission = NoSubmission;
static QUIZ: Quiz = Quiz;
static FORM: Form = Form;
static FILE: File = File;
static LECTURE: Lecture = Lecture;
static SHARED_FILE: SharedFile = SharedFile;

/// The built-in types. The composition root adds the types of modules and installs the result
/// once; it handles the error at startup.
pub fn builtin_types() -> Result<Types, InstallError> {
    Types::new()
        .with(&TEXT)?
        .with(&NO_SUBMISSION)?
        .with(&QUIZ)?
        .with(&FORM)?
        .with(&FILE)?
        .with(&LECTURE)?
        .with(&SHARED_FILE)
}

#[cfg(test)]
mod tests;
