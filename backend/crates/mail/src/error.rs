use common::ApiError;

use crate::model::ModelError;

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error("mail.from \"{0}\" is not a mailbox such as \"Vumlo <no-reply@example.edu>\"")]
    From(String),
    #[error("mail.transport.host \"{0}\" cannot be used as an SMTP host")]
    Host(String),
    #[error(transparent)]
    Invalid(#[from] ModelError),
    /// The plugin made `mail.plugin_hourly_calls` accepted calls in the last hour.
    #[error("this plugin has sent too many mail calls in the last hour")]
    HourlyLimit { retry_after_secs: u64 },
    /// One person already got `mail.recipient_daily` plugin messages in 24 hours.
    #[error("this person has had the most plugin mail allowed in a day")]
    RecipientLimit { retry_after_secs: u64 },
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

impl From<MailError> for ApiError {
    fn from(error: MailError) -> Self {
        match error {
            MailError::Invalid(e) => Self::invalid_field(e.field, e.message),
            MailError::HourlyLimit { retry_after_secs }
            | MailError::RecipientLimit { retry_after_secs } => Self::TooManyRequests {
                retry_after_secs: Some(retry_after_secs),
            },
            MailError::Db(e) => Self::from_db("mail", &e),
            // A configuration error is met at startup; a request never gets one.
            MailError::From(_) | MailError::Host(_) => Self::Internal,
        }
    }
}
