//! The value types of an account and how a password is hashed. The server and `vumlo-cli` both
//! create accounts, so the rules live here once (ADR-0017, ADR-0029).

use std::fmt;
use std::sync::LazyLock;

use argon2::password_hash::PasswordHasher;
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;

use crate::text_enum;

pub const PASSWORD_MAX_LEN: usize = 128;
const EMAIL_MAX_LEN: usize = 254;
const EMAIL_LOCAL_MAX_LEN: usize = 64;
const NAME_MAX_LEN: usize = 100;

/// OWASP's Argon2id minimum: 19 MiB, 2 iterations, 1 lane (ADR-0017).
const ARGON2_MEMORY_KIB: u32 = 19 * 1024;
const ARGON2_ITERATIONS: u32 = 2;
const ARGON2_LANES: u32 = 1;

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
    /// Words that must not appear in a password: email, name, surname, institution, "vumlo".
    pub user_inputs: Vec<String>,
}

impl PasswordPolicy {
    pub fn new(min_len: usize, email: &str, name: &str, surname: &str, institution: &str) -> Self {
        let mut user_inputs = vec![email.to_owned(), "vumlo".to_owned()];
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

/// An Argon2id PHC string, ready to store. Only [`hash`] makes one.
#[derive(Clone, PartialEq, Eq)]
pub struct HashedPassword(String);

impl HashedPassword {
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

/// `N` random bytes in unpadded base64url.
pub fn random_base64url<const N: usize>() -> String {
    let mut bytes = [0u8; N];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

static ARGON2: LazyLock<Result<Argon2<'static>, argon2::Error>> = LazyLock::new(|| {
    let params = Params::new(ARGON2_MEMORY_KIB, ARGON2_ITERATIONS, ARGON2_LANES, None)?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
});

/// The Argon2id instance every hash and check uses.
pub fn argon2() -> Result<&'static Argon2<'static>, String> {
    ARGON2.as_ref().map_err(|e| e.to_string())
}

/// Hashes `password`. It takes tens of milliseconds and 19 MiB: call it on a blocking thread and
/// before opening a transaction.
pub fn hash(password: &str) -> Result<HashedPassword, String> {
    argon2()?
        .hash_password(password.as_bytes())
        .map(|hash| HashedPassword(hash.to_string()))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
