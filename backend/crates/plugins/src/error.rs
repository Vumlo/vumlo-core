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
    #[error(transparent)]
    Forbidden(#[from] AccessError),
    #[error(transparent)]
    Assessment(#[from] AssessmentError),
    #[error(transparent)]
    Teaching(TeachingError),
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

impl From<PluginsError> for ApiError {
    fn from(error: PluginsError) -> Self {
        match error {
            PluginsError::Forbidden(access) => access.into(),
            PluginsError::Assessment(error) => error.into(),
            PluginsError::Teaching(error) => error.into(),
            PluginsError::NotFound(_) => Self::NotFound,
            PluginsError::Db(error) => Self::from_db("plugins", &error),
            // The rest happen at install time, in the CLI, and never in a request.
            other => {
                tracing::error!(error = %other, "a plugin error reached a request");
                Self::Internal
            }
        }
    }
}
