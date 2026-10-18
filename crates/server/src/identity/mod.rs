//! Users, login identities and sessions (ADR-0003, ADR-0017).

mod google;
mod handler;
mod model;
mod repo;
mod routes;
mod service;

pub use google::{Google, GoogleSetupError};
pub use handler::{MailContext, event_handlers};
pub use model::{
    Email, EmailToken, HashedPassword, ModelError, NewSession, NewUser, Password, PasswordPolicy,
    PersonName, Role, SessionInfo, SessionKind, SessionToken, TemporaryPassword, TokenPurpose,
    User, UserStatus,
};
pub use routes::{COOKIE_NAME, SetupError, router};
#[cfg(test)]
pub use service::create_user_unaudited_for_tests;
pub use service::{
    HashGate, IdentityError, LoginThrottle, create_user_from_cli, find_user, find_user_shared,
    find_users, hash_password, hash_password_gated,
};
