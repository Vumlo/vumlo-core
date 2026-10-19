//! HTTP edge of `identity`: extractors, DTO mapping, OpenAPI annotations.

mod admin;
mod auth;
mod extract;
mod google;
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

use super::google::GoogleError;
use super::model::{Role, SessionKind, User, UserStatus};
use super::service::{IdentityError, LoginThrottle, THROTTLE_CAPACITY};
use crate::app::AppState;
use crate::error::ApiError;
use vumlo_config::Settings;

pub use extract::COOKIE_NAME;

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("the login rate limit is not a valid quota")]
    RateLimit,
}

/// How often the limiter forgets clients whose allowance is full again. Without it the state
/// grows by one entry per client address for as long as the server runs (ADR-0038).
const LIMITER_CLEANUP_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// Calls `prune` every `every` until it answers `false`.
fn spawn_cleanup(every: std::time::Duration, mut prune: impl FnMut() -> bool + Send + 'static) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(every).await;
            if !prune() {
                break;
            }
        }
    });
}

/// The client a request counts against: the address from the header the reverse proxy sets
/// (`server.client_ip_header`), else the TCP peer. An IPv4 address is a client of its own; an IPv6
/// address stands for its /64, because whoever holds a /64 can pick any of its 2^64 addresses and
/// would otherwise get a fresh allowance for each (ADR-0038).
#[derive(Debug, Clone)]
struct ClientIp {
    header: Option<header::HeaderName>,
}

/// The key of an address: IPv4-mapped IPv6 is the IPv4 address, other IPv6 keeps its /64.
fn client_key(address: std::net::IpAddr) -> std::net::IpAddr {
    match address.to_canonical() {
        std::net::IpAddr::V6(v6) => {
            std::net::IpAddr::V6(std::net::Ipv6Addr::from(u128::from(v6) & (!0u128 << 64)))
        }
        v4 => v4,
    }
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
            .map(client_key)
            .ok_or(GovernorError::UnableToExtractKey)
    }
}

pub fn router(settings: &Settings) -> Result<OpenApiRouter<AppState>, SetupError> {
    let auth = &settings.auth;
    // A per-client limiter built from the login limits of ADR-0038, with the task that forgets idle
    // clients. Each call makes an allowance of its own. A closure, because the type of the config
    // is not nameable here.
    let client_limiter = || -> Result<_, SetupError> {
        let header = settings
            .server
            .client_ip_header
            .as_deref()
            .map(|name| {
                name.parse::<header::HeaderName>()
                    .map_err(|_| SetupError::RateLimit)
            })
            .transpose()?;
        let config = GovernorConfigBuilder::default()
            .per_second(auth.login_ip_replenish_seconds)
            .burst_size(auth.login_ip_burst)
            .key_extractor(ClientIp { header })
            .finish()
            .ok_or(SetupError::RateLimit)?;
        // The task holds only a weak reference: it ends when the router, and with it the limiter,
        // is dropped.
        let limiter = Arc::downgrade(config.limiter());
        spawn_cleanup(LIMITER_CLEANUP_EVERY, move || {
            let Some(limiter) = limiter.upgrade() else {
                return false;
            };
            limiter.retain_recent();
            limiter.shrink_to_fit();
            true
        });
        Ok(config)
    };
    let throttle = LoginThrottle::with_limits(
        auth.login_account_max_attempts,
        std::time::Duration::from_secs(auth.login_account_window_minutes.saturating_mul(60)),
        THROTTLE_CAPACITY,
    );
    // The same limits, counted per address, for the requests that send mail (ADR-0046).
    let mail_throttle = MailThrottle(LoginThrottle::with_limits(
        auth.login_account_max_attempts,
        std::time::Duration::from_secs(auth.login_account_window_minutes.saturating_mul(60)),
        THROTTLE_CAPACITY,
    ));
    Ok(OpenApiRouter::new()
        .merge(
            OpenApiRouter::new()
                .routes(routes!(auth::login))
                .layer(GovernorLayer::new(client_limiter()?)),
        )
        // Registration and recovery have an allowance of their own, so a lecture hall of students
        // signing in does not use up the allowance of a student signing up (ADR-0046).
        .merge(
            OpenApiRouter::new()
                .routes(routes!(auth::register))
                .routes(routes!(auth::verify_email))
                .routes(routes!(auth::request_password_reset))
                .routes(routes!(auth::confirm_password_reset))
                .routes(routes!(google::start))
                .routes(routes!(google::callback))
                .routes(routes!(google::exchange))
                .layer(GovernorLayer::new(client_limiter()?)),
        )
        .routes(routes!(auth::logout))
        .routes(routes!(me::get_me))
        .routes(routes!(me::change_password))
        .routes(routes!(me::list_sessions))
        .routes(routes!(me::revoke_session))
        .routes(routes!(admin::list_users, admin::create_user))
        .routes(routes!(admin::user_summary))
        .routes(routes!(admin::user_facts))
        .routes(routes!(admin::update_user))
        .routes(routes!(admin::reset_password))
        .layer(Extension(Arc::new(throttle)))
        .layer(Extension(Arc::new(mail_throttle))))
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
            IdentityError::Forbidden(policy) => policy.into(),
            IdentityError::Invalid(model) => model.into(),
            IdentityError::WrongCurrentPassword => {
                Self::invalid_field("current_password", "is incorrect")
            }
            IdentityError::PasswordUnchanged => {
                Self::invalid_field("new_password", "must differ from the current password")
            }
            IdentityError::EmailNotVerified => {
                Self::Forbidden("the email address is not verified yet")
            }
            IdentityError::TokenInvalid => Self::Gone("this link has been used or has expired"),
            IdentityError::EmailDomainNotAllowed => {
                Self::invalid_field("email", "this email domain cannot sign up")
            }
            IdentityError::AppChallengeInvalid => {
                Self::invalid_field("app_challenge", "must be 64 lowercase hex characters")
            }
            IdentityError::GoogleAttemptInvalid => {
                Self::Unauthorized("the sign-in attempt is not valid")
            }
            IdentityError::Google(GoogleError::Rejected) => {
                Self::Unauthorized("Google did not accept the sign-in")
            }
            IdentityError::Google(GoogleError::Unavailable) => {
                Self::ServiceUnavailable("Google sign-in is not available, try again")
            }
            IdentityError::GoogleAccountNotAllowed => {
                Self::Forbidden("this Google account cannot sign in here")
            }
            IdentityError::AccountDisabled => Self::Forbidden("the account is disabled"),
            IdentityError::EmailTaken => Self::Conflict("the email is already in use"),
            IdentityError::ConcurrentChange => {
                Self::Conflict("the account changed while the request was running, try again")
            }
            IdentityError::LastAdmin => {
                Self::Conflict("the last active admin cannot be demoted or disabled")
            }
            IdentityError::OwnsCourses => {
                Self::Conflict("the user owns courses: transfer them to someone else first")
            }
            IdentityError::Teaching(e) => (*e).into(),
            IdentityError::Org(e) => (*e).into(),
            IdentityError::UserNotFound | IdentityError::SessionNotFound => Self::NotFound,
            IdentityError::Hashing => Self::Internal,
            IdentityError::Events(e) => e.into(),
            IdentityError::Db(e) => Self::from_db("identity", &e),
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
            ModelError::Password(reasons) => password_error("new_password", reasons),
        }
    }
}

/// The `422` of a password that breaks the policy, blaming `field`.
pub(super) fn password_error(field: &str, reasons: Vec<String>) -> ApiError {
    ApiError::Unprocessable {
        detail: "the password does not meet the policy".to_owned(),
        errors: reasons
            .into_iter()
            .map(|message| api_types::FieldError {
                field: field.to_owned(),
                message,
            })
            .collect(),
    }
}

/// Counts the requests that make the server send mail, per address (ADR-0046). Its own type, so
/// it is not mixed up with the login throttle in the request extensions.
pub(super) struct MailThrottle(pub(super) LoginThrottle);

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

#[cfg(test)]
mod tests;
