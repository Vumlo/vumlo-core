use common::ApiError;

use super::teaching::TeachingError;
use crate::access::AccessError;
use crate::auth::AuthError;
use crate::org::OrgError;

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
            LearningError::Db(e) => Self::from_db("learning", &e),
        }
    }
}
