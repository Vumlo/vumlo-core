use access::AccessError;
use common::ApiError;
use learning::assessment::AssessmentError;
use learning::teaching::TeachingError;

use crate::secret::SecretError;

#[derive(Debug, thiserror::Error)]
pub enum PluginsError {
    /// The manifest says something this core does not accept. The message names the value.
    #[error("the manifest is not valid: {0}")]
    Manifest(String),
    #[error("the base URL is not valid: {0}")]
    BaseUrl(String),
    #[error("cannot read the manifest: {0}")]
    Fetch(String),
    /// `plugins.secret_key` is not set, so no secret can be derived.
    #[error("plugins.secret_key is not set")]
    NoKey,
    /// An administrator granted a scope the plugin did not ask for.
    #[error("the plugin did not ask for the scope `{0}`")]
    NotRequested(String),
    #[error("a plugin with the id `{0}` is installed already")]
    AlreadyInstalled(String),
    #[error("there is no plugin `{0}`")]
    NotFound(String),
    /// The plugin declared no such setting.
    #[error("{plugin} has no setting `{key}`")]
    NoSuchSetting { plugin: String, key: String },
    /// A course setting was addressed without a course, or an institution setting with one.
    #[error("the setting `{key}` is {scope}; {hint}")]
    WrongScope {
        key: String,
        scope: &'static str,
        hint: &'static str,
    },
    /// The value does not meet the setting's spec. The message does not repeat the value.
    #[error("the value of `{key}` {why}")]
    BadSetting { key: String, why: String },
    /// A setting was addressed to a course that is not there.
    #[error("there is no such course")]
    CourseGone,
    /// What a report is about (a submitted attempt, an assignment, a course) is not there.
    #[error("there is no such submitted attempt, assignment or course")]
    SubjectGone,
    /// There is no report of the plugin under that key.
    #[error("there is no such report")]
    ReportGone,
    /// The plugin already keeps as many reports on the subject as it may.
    #[error("a plugin may keep at most {0} reports on one subject")]
    TooManyReports(i64),
    /// The configured allowance of calls per token is not one the limiter can use.
    #[error("the plugin API rate limit is not a valid quota")]
    RateLimit,
    #[error(transparent)]
    Forbidden(#[from] AccessError),
    #[error(transparent)]
    Assessment(#[from] AssessmentError),
    #[error(transparent)]
    Teaching(TeachingError),
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error(transparent)]
    Events(#[from] events::EventsError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

impl From<PluginsError> for ApiError {
    fn from(error: PluginsError) -> Self {
        match error {
            PluginsError::Forbidden(access) => access.into(),
            PluginsError::Assessment(error) => error.into(),
            PluginsError::Teaching(error) => error.into(),
            PluginsError::NotFound(_)
            | PluginsError::SubjectGone
            | PluginsError::ReportGone
            | PluginsError::CourseGone => Self::NotFound,
            PluginsError::TooManyReports(_) => Self::invalid_field("key", error.to_string()),
            PluginsError::Db(error) => Self::from_db("plugins", &error),
            // The rest happen at install time, in the CLI, and never in a request.
            other => {
                tracing::error!(error = %other, "a plugin error reached a request");
                Self::Internal
            }
        }
    }
}
