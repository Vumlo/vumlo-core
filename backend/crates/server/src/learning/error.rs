use common::ApiError;

use super::teaching::TeachingError;
use crate::org::OrgError;
use access::AccessError;
use auth::AuthError;

/// What goes wrong in a flow that crosses modules: the error of the module that failed.
#[derive(Debug, thiserror::Error)]
pub enum LearningError {
    #[error(transparent)]
    Access(#[from] AccessError),
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Org(#[from] OrgError),
    #[error(transparent)]
    Teaching(#[from] TeachingError),
    /// A teacher or admin cannot become a student while they own a course (ADR-0037).
    #[error("the user owns courses")]
    OwnsCourses,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

impl From<LearningError> for ApiError {
    fn from(error: LearningError) -> Self {
        match error {
            LearningError::Access(e) => e.into(),
            LearningError::Auth(e) => e.into(),
            LearningError::Org(e) => e.into(),
            LearningError::Teaching(e) => e.into(),
            LearningError::OwnsCourses => {
                Self::Conflict("the user owns courses: transfer them to someone else first")
            }
            LearningError::Db(e) => Self::from_db("learning", &e),
        }
    }
}
