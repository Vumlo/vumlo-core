//! Users, login identities, sessions and the mail that goes with them (ADR-0003, ADR-0017).

mod google;
mod handler;
mod mail;
mod model;
mod repo;
mod routes;
mod service;
mod state;
#[cfg(feature = "testing")]
mod test_support;

pub use google::{Google, GoogleSetupError};
pub use handler::{MailContext, event_handlers};
pub use mail::{MailError, Mailer};
pub use model::{
    Email, EmailToken, HashedPassword, ModelError, NewSession, NewUser, Password, PasswordPolicy,
    PersonName, SessionInfo, SessionKind, SessionToken, TemporaryPassword, TokenPurpose, User,
    UserStatus, UserUpdate, hash,
};
pub use routes::{
    COOKIE_NAME, SetupError, api_role, api_status, api_user, authenticate_request, domain_role,
    domain_status, router,
};
pub use service::{
    AppliedUserUpdate, AuthError, HashGate, LoginThrottle, apply_user_update, create_user_from_cli,
    find_user, find_user_shared, find_users, hash_password, hash_password_gated, last_login_at,
};
pub use state::AuthState;
#[cfg(feature = "testing")]
pub use test_support::create_user_unaudited_for_tests;
