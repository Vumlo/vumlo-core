mod account;

use std::fmt;

use access::Audience;
use chrono::{DateTime, Duration, Utc};
use common::{Role, text_enum};
// The value types of an account are shared with `vumlo-cli`, which creates accounts too.
pub(crate) use account::random_base64url;
pub use account::{
    Email, HashedPassword, ModelError, NewUser, Password, PasswordPolicy, PersonName,
    TemporaryPassword, argon2, hash,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// 32 random bytes in unpadded base64url.
const TOKEN_LEN: usize = 43;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserStatus {
    Active,
    Disabled,
}
text_enum!(UserStatus { Active => "active", Disabled => "disabled" });

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Web,
    App,
    Cli,
    Pat,
}
text_enum!(SessionKind { Web => "web", App => "app", Cli => "cli", Pat => "pat" });

/// How long a session lives (ADR-0017, ADR-0095).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lifetimes {
    /// From creation to the first expiry. `Pat` is the maximum; personal access tokens will choose
    /// a shorter one when they exist.
    pub initial: Duration,
    /// Extended by this much on activity. `None`: the expiry is absolute.
    pub slide: Option<Duration>,
    /// A sliding session never lives past its creation plus this. `None`: no cap.
    pub cap: Option<Duration>,
    /// A session seen more recently than this is not written again, so concurrent requests write
    /// at most once.
    pub refresh_gap: Duration,
}

impl SessionKind {
    pub fn lifetimes(self, audience: Audience) -> Lifetimes {
        let hour = Duration::hours(1);
        match (self, audience) {
            // The console: 30 minutes idle, 8 hours at most, refreshed after a minute.
            (_, Audience::Console) => Lifetimes {
                initial: Duration::minutes(30),
                slide: Some(Duration::minutes(30)),
                cap: Some(Duration::hours(8)),
                refresh_gap: Duration::minutes(1),
            },
            (Self::Web, Audience::App) => Lifetimes {
                initial: Duration::days(30),
                slide: Some(Duration::days(30)),
                cap: None,
                refresh_gap: hour,
            },
            (Self::App, Audience::App) => Lifetimes {
                initial: Duration::days(90),
                slide: Some(Duration::days(90)),
                cap: None,
                refresh_gap: hour,
            },
            (Self::Cli, Audience::App) => Lifetimes {
                initial: Duration::days(90),
                slide: None,
                cap: None,
                refresh_gap: hour,
            },
            (Self::Pat, Audience::App) => Lifetimes {
                initial: Duration::days(365),
                slide: None,
                cap: None,
                refresh_gap: hour,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub surname: String,
    pub role: Role,
    pub status: UserStatus,
    pub must_change_password: bool,
    pub created_at: DateTime<Utc>,
}

/// People by institution role, and how many are disabled (ADR-0053).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleCounts {
    pub all: i64,
    pub student: i64,
    pub teacher: i64,
    pub admin: i64,
    pub disabled: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub id: Uuid,
    pub kind: SessionKind,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Which users `list_users` returns. Empty means all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserFilter {
    pub role: Option<Role>,
    pub status: Option<UserStatus>,
    /// Case-insensitive substring of the email.
    pub email_contains: Option<String>,
}

/// Fields an admin may change. `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserUpdate {
    pub name: Option<PersonName>,
    pub surname: Option<PersonName>,
    pub role: Option<Role>,
    pub status: Option<UserStatus>,
}

/// A new session as the client receives it: the raw token is shown once and never stored.
#[derive(Debug, Clone)]
pub struct NewSession {
    pub token: SessionToken,
    pub kind: SessionKind,
    pub expires_at: DateTime<Utc>,
}

/// What an emailed token proves (ADR-0046).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenPurpose {
    /// The person controls the mailbox of the address.
    Verify,
    /// The person may choose a new password.
    Reset,
}
text_enum!(TokenPurpose { Verify => "verify", Reset => "reset" });

impl TokenPurpose {
    /// How long a token of this purpose can be used (ADR-0046).
    pub fn lifetime(self) -> Duration {
        match self {
            Self::Verify => Duration::hours(24),
            Self::Reset => Duration::hours(1),
        }
    }
}

/// The secret in a verification or reset link. Only its SHA-256 is stored, and the value exists
/// only in the mail.
#[derive(Clone, PartialEq, Eq)]
pub struct EmailToken(String);

impl EmailToken {
    pub fn generate() -> Self {
        Self(random_base64url::<32>())
    }

    /// Accepts only well-formed tokens, so junk never reaches the database.
    pub fn from_presented(raw: &str) -> Option<Self> {
        let well_formed = raw.len() == TOKEN_LEN
            && raw
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        well_formed.then(|| Self(raw.to_owned()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn hash(&self) -> [u8; 32] {
        Sha256::digest(self.0.as_bytes()).into()
    }
}

impl fmt::Debug for EmailToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EmailToken(***)")
    }
}

/// The random value that ties a started Google sign-in to the browser that started it: it is set as
/// a cookie by `start` and must come back with the callback (ADR-0047). Only its SHA-256 is stored.
#[derive(Clone, PartialEq, Eq)]
pub struct BrowserBinding(String);

impl BrowserBinding {
    pub fn generate() -> Self {
        Self(random_base64url::<32>())
    }

    pub fn from_presented(raw: &str) -> Option<Self> {
        EmailToken::from_presented(raw).map(|token| Self(token.0))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn hash(&self) -> [u8; 32] {
        Sha256::digest(self.0.as_bytes()).into()
    }
}

impl fmt::Debug for BrowserBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BrowserBinding(***)")
    }
}

/// An opaque session credential. Only its SHA-256 is stored.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionToken(String);

impl SessionToken {
    pub fn generate() -> Self {
        Self(random_base64url::<32>())
    }

    /// Accepts only well-formed tokens, so junk never reaches the database.
    pub fn from_presented(raw: &str) -> Option<Self> {
        let well_formed = raw.len() == TOKEN_LEN
            && raw
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        well_formed.then(|| Self(raw.to_owned()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn hash(&self) -> [u8; 32] {
        Sha256::digest(self.0.as_bytes()).into()
    }
}

impl fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionToken(***)")
    }
}

#[cfg(test)]
mod tests;
