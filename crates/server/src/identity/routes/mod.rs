//! HTTP edge of `identity`: extractors, DTO mapping, OpenAPI annotations.

mod admin;
mod auth;
mod extract;
mod me;

use std::sync::Arc;

use api_types::LoginClient;
use axum::Extension;
use axum::http::{HeaderValue, header};
use tower_governor::GovernorError;
use tower_governor::GovernorLayer;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::KeyExtractor;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::model::{Role, SessionKind, User, UserStatus};
use super::service::{IdentityError, LoginThrottle};
use crate::app::AppState;
use crate::error::ApiError;
use crate::settings::Settings;

pub use extract::COOKIE_NAME;

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("the login rate limit is not a valid quota")]
    RateLimit,
}

/// Burst of 50 and one more per second, per client IP: generous because a whole school may
/// share one address. The strict limit is per account (`LoginThrottle`).
const LOGIN_IP_BURST: u32 = 50;
const LOGIN_IP_PERIOD_SECS: u64 = 1;

/// Rate-limit key: the header the reverse proxy sets (`server.client_ip_header`), else the
/// TCP peer address.
#[derive(Debug, Clone)]
struct ClientIp {
    header: Option<header::HeaderName>,
}

impl KeyExtractor for ClientIp {
    type Key = std::net::IpAddr;

    fn extract<T>(&self, req: &axum::http::Request<T>) -> Result<Self::Key, GovernorError> {
        let from_header = self
            .header
            .as_ref()
            .and_then(|name| req.headers().get(name))
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse().ok());
        from_header
            .or_else(|| {
                req.extensions()
                    .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                    .map(|info| info.0.ip())
            })
            .ok_or(GovernorError::UnableToExtractKey)
    }
}

pub fn router(settings: &Settings) -> Result<OpenApiRouter<AppState>, SetupError> {
    let header = settings
        .server
        .client_ip_header
        .as_deref()
        .map(|name| {
            name.parse::<header::HeaderName>()
                .map_err(|_| SetupError::RateLimit)
        })
        .transpose()?;
    let login_limit = GovernorConfigBuilder::default()
        .per_second(LOGIN_IP_PERIOD_SECS)
        .burst_size(LOGIN_IP_BURST)
        .key_extractor(ClientIp { header })
        .finish()
        .ok_or(SetupError::RateLimit)?;
    Ok(OpenApiRouter::new()
        .merge(
            OpenApiRouter::new()
                .routes(routes!(auth::login))
                .layer(GovernorLayer::new(login_limit)),
        )
        .routes(routes!(auth::logout))
        .routes(routes!(me::get_me))
        .routes(routes!(me::change_password))
        .routes(routes!(me::list_sessions))
        .routes(routes!(me::revoke_session))
        .routes(routes!(admin::list_users, admin::create_user))
        .routes(routes!(admin::update_user))
        .routes(routes!(admin::reset_password))
        .layer(Extension(Arc::new(LoginThrottle::default()))))
}

impl From<IdentityError> for ApiError {
    fn from(error: IdentityError) -> Self {
        match error {
            IdentityError::InvalidCredentials => Self::Unauthorized("invalid email or password"),
            IdentityError::TooManyAttempts { retry_after_secs } => Self::TooManyRequests {
                retry_after_secs: Some(retry_after_secs),
            },
            IdentityError::Unauthenticated => Self::Unauthorized("authentication required"),
            IdentityError::PasswordChangeRequired => {
                Self::Forbidden("the password must be changed first")
            }
            IdentityError::OriginMismatch => Self::Forbidden("origin not allowed"),
            IdentityError::Forbidden(_) => Self::Forbidden("not allowed"),
            IdentityError::Invalid(model) => model.into(),
            IdentityError::WrongCurrentPassword => {
                Self::invalid_field("current_password", "is incorrect")
            }
            IdentityError::PasswordUnchanged => {
                Self::invalid_field("new_password", "must differ from the current password")
            }
            IdentityError::EmailTaken => Self::Conflict("the email is already in use"),
            IdentityError::ConcurrentChange => {
                Self::Conflict("the account changed while the request was running, try again")
            }
            IdentityError::LastAdmin => {
                Self::Conflict("the last active admin cannot be demoted or disabled")
            }
            IdentityError::UserNotFound | IdentityError::SessionNotFound => Self::NotFound,
            IdentityError::Hashing => Self::Internal,
            IdentityError::Db(e) => {
                tracing::error!(error = %e, "identity: database error");
                Self::Internal
            }
        }
    }
}

impl From<super::model::ModelError> for ApiError {
    fn from(error: super::model::ModelError) -> Self {
        use super::model::ModelError;
        match error {
            ModelError::Email => Self::invalid_field("email", error.to_string()),
            ModelError::Name => Self::invalid_field("name", error.to_string()),
            ModelError::Surname => Self::invalid_field("surname", error.to_string()),
            ModelError::Password(reasons) => Self::Unprocessable {
                detail: "the password does not meet the policy".to_owned(),
                errors: reasons
                    .into_iter()
                    .map(|message| api_types::FieldError {
                        field: "new_password".to_owned(),
                        message,
                    })
                    .collect(),
            },
        }
    }
}

pub(super) fn api_role(role: Role) -> api_types::Role {
    match role {
        Role::Admin => api_types::Role::Admin,
        Role::Teacher => api_types::Role::Teacher,
        Role::Student => api_types::Role::Student,
    }
}

pub(super) fn domain_role(role: api_types::Role) -> Role {
    match role {
        api_types::Role::Admin => Role::Admin,
        api_types::Role::Teacher => Role::Teacher,
        api_types::Role::Student => Role::Student,
    }
}

pub(super) fn domain_status(status: api_types::UserStatus) -> UserStatus {
    match status {
        api_types::UserStatus::Active => UserStatus::Active,
        api_types::UserStatus::Disabled => UserStatus::Disabled,
    }
}

pub(super) fn api_status(status: UserStatus) -> api_types::UserStatus {
    match status {
        UserStatus::Active => api_types::UserStatus::Active,
        UserStatus::Disabled => api_types::UserStatus::Disabled,
    }
}

pub(super) fn api_kind(kind: SessionKind) -> api_types::SessionKind {
    match kind {
        SessionKind::Web => api_types::SessionKind::Web,
        SessionKind::App => api_types::SessionKind::App,
        SessionKind::Cli => api_types::SessionKind::Cli,
        SessionKind::Pat => api_types::SessionKind::Pat,
    }
}

pub(super) fn api_user(user: User) -> api_types::User {
    api_types::User {
        id: user.id,
        email: user.email,
        name: user.name,
        surname: user.surname,
        role: api_role(user.role),
        status: api_status(user.status),
        must_change_password: user.must_change_password,
        created_at: user.created_at,
    }
}

pub(super) fn session_kind(client: LoginClient) -> SessionKind {
    match client {
        LoginClient::Web => SessionKind::Web,
        LoginClient::App => SessionKind::App,
    }
}

/// Responses that carry a credential must not be cached.
pub(super) fn no_store() -> (header::HeaderName, HeaderValue) {
    (header::CACHE_CONTROL, HeaderValue::from_static("no-store"))
}
