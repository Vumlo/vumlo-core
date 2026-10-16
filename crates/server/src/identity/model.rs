use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, Utc};
use rand::Rng;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::text_enum::text_enum;

pub const PASSWORD_MAX_LEN: usize = 128;
const EMAIL_MAX_LEN: usize = 254;
const EMAIL_LOCAL_MAX_LEN: usize = 64;
const NAME_MAX_LEN: usize = 100;
/// 32 random bytes in unpadded base64url.
const TOKEN_LEN: usize = 43;

/// A value that failed validation. Each variant maps to one field of the API.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    #[error("not a valid email address")]
    Email,
    #[error("must be 1 to {NAME_MAX_LEN} characters")]
    Name,
    #[error("must be 1 to {NAME_MAX_LEN} characters")]
    Surname,
    /// Human-readable reasons, the first being zxcvbn's warning when it has one.
    #[error("password rejected")]
    Password(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Admin,
    Teacher,
    Student,
}
text_enum!(Role { Admin => "admin", Teacher => "teacher", Student => "student" });

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

impl SessionKind {
    /// Lifetime of a session when it is created (ADR-0017). `Pat` is the maximum; personal
    /// access tokens will choose a shorter one when they exist.
    pub fn initial_lifetime(self) -> Duration {
        match self {
            Self::Web => Duration::days(30),
            Self::App | Self::Cli => Duration::days(90),
            Self::Pat => Duration::days(365),
        }
    }

    /// Lifetime of a sliding session, extended on activity. `None`: the expiry is absolute (ADR-0017).
    pub fn sliding_lifetime(self) -> Option<Duration> {
        match self {
            Self::Web => Some(Duration::days(30)),
            Self::App => Some(Duration::days(90)),
            Self::Cli | Self::Pat => None,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewUser {
    pub email: Email,
    pub name: PersonName,
    pub surname: PersonName,
    pub role: Role,
}

/// A trimmed, lowercased address with basic syntax checks. Delivery is proven by verification, not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Email(String);

impl Email {
    pub fn parse(raw: &str) -> Result<Self, ModelError> {
        let email = raw.trim().to_lowercase();
        let (local, domain) = email.split_once('@').ok_or(ModelError::Email)?;
        let local_ok = !local.is_empty()
            && local.chars().count() <= EMAIL_LOCAL_MAX_LEN
            && !local.starts_with('.')
            && !local.ends_with('.')
            && !local.contains("..")
            && local.chars().all(|c| {
                !c.is_whitespace()
                    && !c.is_control()
                    && !matches!(
                        c,
                        '(' | ')' | '<' | '>' | '[' | ']' | ':' | ';' | '@' | '\\' | ',' | '"'
                    )
            });
        let labels: Vec<&str> = domain.split('.').collect();
        let domain_ok = labels.len() >= 2
            && labels.iter().all(|label| {
                !label.is_empty()
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label.chars().all(|c| c.is_alphanumeric() || c == '-')
            });
        if local_ok && domain_ok && email.chars().count() <= EMAIL_MAX_LEN {
            Ok(Self(email))
        } else {
            Err(ModelError::Email)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A trimmed name or surname of 1 to 100 characters without control characters (ADR-0029). Which
/// of the two failed is decided by the constructor, because each maps to its own API field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonName(String);

impl PersonName {
    pub fn name(raw: &str) -> Result<Self, ModelError> {
        Self::parse(raw).ok_or(ModelError::Name)
    }

    pub fn surname(raw: &str) -> Result<Self, ModelError> {
        Self::parse(raw).ok_or(ModelError::Surname)
    }

    fn parse(raw: &str) -> Option<Self> {
        let text = raw.trim();
        let len = text.chars().count();
        ((1..=NAME_MAX_LEN).contains(&len) && !text.chars().any(char::is_control))
            .then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a password must satisfy (ADR-0017).
#[derive(Debug, Clone)]
pub struct PasswordPolicy {
    pub min_len: usize,
    /// Words that must not appear in a password: email, name, surname, institution, "learnly".
    pub user_inputs: Vec<String>,
}

impl PasswordPolicy {
    pub fn new(min_len: usize, email: &str, name: &str, surname: &str, institution: &str) -> Self {
        let mut user_inputs = vec![email.to_owned(), "learnly".to_owned()];
        let (local, domain) = email.split_once('@').unwrap_or_default();
        user_inputs.push(local.to_owned());
        user_inputs.extend(domain.split('.').map(str::to_owned));
        for text in [name, surname, institution] {
            user_inputs.push(text.to_owned());
            user_inputs.extend(text.split_whitespace().map(str::to_owned));
        }
        Self {
            min_len,
            user_inputs,
        }
    }
}

/// A password that satisfies the policy. The value never appears in `Debug` output.
#[derive(Clone, PartialEq, Eq)]
pub struct Password(String);

impl Password {
    pub fn parse(raw: String, policy: &PasswordPolicy) -> Result<Self, ModelError> {
        let len = raw.chars().count();
        if len < policy.min_len {
            return Err(ModelError::Password(vec![format!(
                "must be at least {} characters",
                policy.min_len
            )]));
        }
        if len > PASSWORD_MAX_LEN {
            return Err(ModelError::Password(vec![format!(
                "must be at most {PASSWORD_MAX_LEN} characters"
            )]));
        }
        let inputs: Vec<&str> = policy.user_inputs.iter().map(String::as_str).collect();
        let entropy = zxcvbn::zxcvbn(&raw, &inputs);
        if entropy.score() < zxcvbn::Score::Three {
            let mut reasons = Vec::new();
            if let Some(feedback) = entropy.feedback() {
                reasons.extend(feedback.warning().map(|w| w.to_string()));
                reasons.extend(feedback.suggestions().iter().map(|s| s.to_string()));
            }
            if reasons.is_empty() {
                reasons.push("is too easy to guess".to_owned());
            }
            return Err(ModelError::Password(reasons));
        }
        Ok(Self(raw))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(***)")
    }
}

/// An Argon2id PHC string, ready to store. Only `service::hash_password` makes one.
#[derive(Clone, PartialEq, Eq)]
pub struct HashedPassword(String);

impl HashedPassword {
    pub(super) fn new(phc: String) -> Self {
        Self(phc)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for HashedPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HashedPassword(***)")
    }
}

/// A generated password for admin-created accounts: shown once, then must be changed.
#[derive(Clone, PartialEq, Eq)]
pub struct TemporaryPassword(String);

impl TemporaryPassword {
    pub fn generate() -> Self {
        Self(random_base64url::<16>())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for TemporaryPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TemporaryPassword(***)")
    }
}

/// A new session as the client receives it: the raw token is shown once and never stored.
#[derive(Debug, Clone)]
pub struct NewSession {
    pub token: SessionToken,
    pub kind: SessionKind,
    pub expires_at: DateTime<Utc>,
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

fn random_base64url<const N: usize>() -> String {
    let mut bytes = [0u8; N];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> PasswordPolicy {
        PasswordPolicy::new(
            12,
            "anna.kowalska@school.edu",
            "Anna",
            "Kowalska",
            "Example University",
        )
    }

    fn rejection(raw: &str) -> Vec<String> {
        match Password::parse(raw.to_owned(), &policy()) {
            Err(ModelError::Password(reasons)) => reasons,
            other => panic!("expected a rejection, got {other:?}"),
        }
    }

    #[test]
    fn email_is_trimmed_and_lowercased() {
        assert_eq!(
            Email::parse("  Anna.K+lms@School.EDU ").unwrap().as_str(),
            "anna.k+lms@school.edu"
        );
    }

    #[test]
    fn email_rejects_bad_syntax() {
        for bad in [
            "",
            "plain",
            "@school.edu",
            "a@",
            "a@school",
            "a b@school.edu",
            "a@@school.edu",
            "a@school..edu",
            "a@-school.edu",
            ".a@school.edu",
            "a..b@school.edu",
            "<a>@school.edu",
            "a@school.edu.",
            "a@sch ool.edu",
        ] {
            assert_eq!(Email::parse(bad), Err(ModelError::Email), "{bad:?}");
        }
        let long_local = format!("{}@school.edu", "a".repeat(65));
        assert!(Email::parse(&long_local).is_err());
        let long_total = format!("a@{}.edu", "b".repeat(250));
        assert!(Email::parse(&long_total).is_err());
    }

    #[test]
    fn names_are_trimmed_and_bounded_and_each_reports_its_own_field() {
        assert_eq!(PersonName::name("  Anna  ").unwrap().as_str(), "Anna");
        assert_eq!(
            PersonName::surname(" Kowalska ").unwrap().as_str(),
            "Kowalska"
        );
        for bad in ["   ", "", "a\nb", &"ł".repeat(101)] {
            assert_eq!(PersonName::name(bad), Err(ModelError::Name), "{bad:?}");
            assert_eq!(
                PersonName::surname(bad),
                Err(ModelError::Surname),
                "{bad:?}"
            );
        }
        assert!(PersonName::name(&"ł".repeat(100)).is_ok());
    }

    #[test]
    fn eleven_chars_rejected_and_the_limit_is_configurable() {
        let reasons = rejection("Tr0ub4dor&3");
        assert_eq!(reasons, ["must be at least 12 characters"]);
        let relaxed = PasswordPolicy {
            min_len: 8,
            ..policy()
        };
        assert!(Password::parse("vermilion-quartz-lantern".to_owned(), &relaxed).is_ok());
    }

    #[test]
    fn common_password_rejected_with_feedback() {
        let reasons = rejection("password1234");
        assert!(!reasons.is_empty());
        assert!(reasons.iter().all(|r| !r.contains("password1234")));
    }

    #[test]
    fn password_built_from_user_inputs_is_rejected() {
        rejection("annakowalska");
        let without_inputs = PasswordPolicy {
            user_inputs: Vec::new(),
            ..policy()
        };
        assert!(
            Password::parse("annakowalska".to_owned(), &without_inputs).is_ok(),
            "the same password is only weak because it matches the user's own data"
        );
    }

    #[test]
    fn long_passphrase_with_spaces_and_unicode_is_accepted() {
        let ok = Password::parse(
            "zażółć gęślą jaźń — wieczorem nad łąką".to_owned(),
            &policy(),
        );
        assert!(ok.is_ok(), "{ok:?}");
        assert!(Password::parse("correct horse battery staple".to_owned(), &policy()).is_ok());
    }

    #[test]
    fn password_longer_than_128_chars_is_rejected() {
        let long = "correct horse battery staple ".repeat(5);
        assert!(long.chars().count() > PASSWORD_MAX_LEN);
        assert_eq!(rejection(&long), ["must be at most 128 characters"]);
    }

    #[test]
    fn secrets_are_never_printed() {
        let password =
            Password::parse("correct horse battery staple".to_owned(), &policy()).unwrap();
        let token = SessionToken::generate();
        let temp = TemporaryPassword::generate();
        for printed in [
            format!("{password:?}"),
            format!("{token:?}"),
            format!("{temp:?}"),
        ] {
            assert!(printed.contains("***"), "{printed}");
        }
        assert!(!format!("{token:?}").contains(token.expose()));
        assert!(!format!("{temp:?}").contains(temp.expose()));
    }

    #[test]
    fn session_tokens_are_43_url_safe_chars_and_unique() {
        let a = SessionToken::generate();
        let b = SessionToken::generate();
        assert_eq!(a.expose().len(), 43);
        assert!(
            a.expose()
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        );
        assert_ne!(a, b);
        assert_eq!(SessionToken::from_presented(a.expose()), Some(a));
    }

    #[test]
    fn malformed_presented_tokens_are_rejected() {
        let good = SessionToken::generate();
        for bad in [
            "",
            "short",
            &format!("{}=", good.expose()),
            &format!("{} ", &good.expose()[..42]),
            &good.expose().replace(&good.expose()[..1], "+"),
        ] {
            assert_eq!(SessionToken::from_presented(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn token_hash_is_sha256_of_the_token_text() {
        let token = SessionToken::from_presented(&"A".repeat(43)).unwrap();
        let expected: [u8; 32] = Sha256::digest("A".repeat(43).as_bytes()).into();
        assert_eq!(token.hash(), expected);
        assert_ne!(token.hash(), SessionToken::generate().hash());
    }

    #[test]
    fn temporary_password_has_128_bits_of_entropy_as_22_chars() {
        let temp = TemporaryPassword::generate();
        assert_eq!(temp.expose().len(), 22);
        assert_ne!(temp, TemporaryPassword::generate());
    }

    #[test]
    fn text_enums_round_trip() {
        for role in [Role::Admin, Role::Teacher, Role::Student] {
            assert_eq!(role.as_str().parse::<Role>(), Ok(role));
        }
        for kind in [
            SessionKind::Web,
            SessionKind::App,
            SessionKind::Cli,
            SessionKind::Pat,
        ] {
            assert_eq!(kind.as_str().parse::<SessionKind>(), Ok(kind));
        }
        assert!("owner".parse::<Role>().is_err());
    }

    #[test]
    fn only_web_and_app_sessions_slide() {
        assert_eq!(
            SessionKind::Web.sliding_lifetime(),
            Some(Duration::days(30))
        );
        assert_eq!(
            SessionKind::App.sliding_lifetime(),
            Some(Duration::days(90))
        );
        assert_eq!(SessionKind::Cli.sliding_lifetime(), None);
        assert_eq!(SessionKind::Pat.sliding_lifetime(), None);
    }
}
