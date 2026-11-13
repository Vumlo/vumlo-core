use common::ApiError;

use super::assessment::AssessmentError;
use super::content::ContentError;
use super::files::FileError;
use super::gradebook::GradebookError;
use access::AccessError;
use auth::AuthError;
use org::OrgError;
use teaching::TeachingError;

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
    Content(#[from] ContentError),
    #[error(transparent)]
    Gradebook(#[from] GradebookError),
    #[error(transparent)]
    Assessment(#[from] AssessmentError),
    /// The check of the files a write names failed (ADR-0069); the verdict itself is not an error.
    #[error(transparent)]
    Files(#[from] FileError),
    /// The assignment, or an assignment below the module, has a grade (ADR-0028).
    #[error("it has grades: clear them before deleting")]
    HasGrades,
    /// The assignment, or an assignment below the module, has a submitted attempt (ADR-0035).
    #[error("it has submitted work")]
    HasSubmissions,
    /// A grade has a value in the course, so it cannot be deleted (ADR-0048).
    #[error("the course has grades")]
    CourseHasGrades,
    /// An attempt was submitted in the course, so it cannot be deleted (ADR-0048).
    #[error("the course has submitted work")]
    CourseHasSubmissions,
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
            LearningError::Content(e) => e.into(),
            LearningError::Gradebook(e) => e.into(),
            LearningError::Assessment(e) => e.into(),
            LearningError::Files(e) => e.into(),
            LearningError::HasGrades => Self::Conflict("it has grades: clear them before deleting"),
            LearningError::HasSubmissions => {
                Self::Conflict("it has submitted work: it cannot be deleted")
            }
            LearningError::CourseHasGrades => Self::Conflict(
                "grades were given in this course, so it cannot be deleted: archive it instead",
            ),
            LearningError::CourseHasSubmissions => Self::Conflict(
                "work was submitted in this course, so it cannot be deleted: archive it instead",
            ),
            LearningError::OwnsCourses => {
                Self::Conflict("the user owns courses: transfer them to someone else first")
            }
            LearningError::Db(e) => Self::from_db("learning", &e),
        }
    }
}
