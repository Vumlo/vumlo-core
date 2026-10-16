//! Users, login identities and sessions (ADR-0003, ADR-0017).

mod model;
mod repo;
mod routes;
mod service;

pub use model::{
    Email, HashedPassword, ModelError, NewSession, NewUser, Password, PasswordPolicy, PersonName,
    Role, SessionInfo, SessionKind, SessionToken, TemporaryPassword, User, UserStatus,
};
pub use routes::{COOKIE_NAME, SetupError, router};
pub use service::{IdentityError, LoginThrottle, create_user, find_user, hash_password};
