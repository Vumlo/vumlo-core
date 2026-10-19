//! Business rules. Every function that acts for a user authorizes first, takes the connection
//! it runs on and never begins or commits a transaction. Password hashing is exposed separately
//! so callers can do it before they open one.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use argon2::password_hash::PasswordVerifier;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use uuid::Uuid;

use super::google::{Begin, GoogleProfile};
use super::model::{
    BrowserBinding, Email, EmailToken, HashedPassword, ModelError, NewSession, NewUser, Password,
    PasswordPolicy, PersonName, Role, RoleCounts, SessionInfo, SessionKind, SessionToken,
    TemporaryPassword, TokenPurpose, User, UserFacts, UserFilter, UserStatus, UserUpdate,
};
use super::repo::{self, LoginRecord};
use crate::actor::Actor;
use crate::events::{self, AuditEntry, Event, EventsError};
use crate::org::{self, OrgError};
use crate::pagination::{Page, PageParams};
use crate::policy::{self, Action, PolicyError};
use crate::teaching::{self, TeachingError};

const LOGIN_MAX_ATTEMPTS: u32 = 10;
const LOGIN_WINDOW: Duration = Duration::from_secs(15 * 60);
pub(super) const THROTTLE_CAPACITY: usize = 100_000;

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    /// Wrong email, wrong password, disabled user: callers must not learn which.
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("too many failed attempts")]
    TooManyAttempts { retry_after_secs: u64 },
    #[error("no valid session")]
    Unauthenticated,
    #[error("the password must be changed first")]
    PasswordChangeRequired,
    #[error("the request origin is not allowed")]
    OriginMismatch,
    #[error(transparent)]
    Forbidden(#[from] PolicyError),
    #[error(transparent)]
    Invalid(#[from] ModelError),
    #[error("the current password is wrong")]
    WrongCurrentPassword,
    #[error("the new password equals the current one")]
    PasswordUnchanged,
    /// The password was right, but the address is not verified yet (ADR-0046).
    #[error("the email address is not verified")]
    EmailNotVerified,
    /// A verification or reset token that is unknown, used, expired or malformed.
    #[error("the link is not valid any more")]
    TokenInvalid,
    #[error("the email domain is not allowed to register")]
    EmailDomainNotAllowed,
    /// The Google sign-in attempt is unknown, used, expired or from another browser (ADR-0047).
    #[error("the sign-in attempt is not valid")]
    GoogleAttemptInvalid,
    /// The `app_challenge` of a start request is not 64 lowercase hex characters.
    #[error("the app challenge is not valid")]
    AppChallengeInvalid,
    #[error(transparent)]
    Google(#[from] super::google::GoogleError),
    /// The Google account has no verified email, or its domain may not sign up.
    #[error("this Google account cannot sign in here")]
    GoogleAccountNotAllowed,
    #[error("the account is disabled")]
    AccountDisabled,
    #[error("the email is already in use")]
    EmailTaken,
    #[error("the account changed while the request was running")]
    ConcurrentChange,
    #[error("the last active admin cannot be demoted or disabled")]
    LastAdmin,
    /// A teacher or admin cannot become a student while they own a course (ADR-0037).
    #[error("the user owns courses")]
    OwnsCourses,
    #[error(transparent)]
    Teaching(Box<TeachingError>),
    #[error(transparent)]
    Org(Box<OrgError>),
    #[error("user not found")]
    UserNotFound,
    #[error("session not found")]
    SessionNotFound,
    #[error("password hashing failed")]
    Hashing,
    #[error(transparent)]
    Events(#[from] EventsError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// The database access every identity path starts from. Maps a unique violation on `users.email`.
fn email_taken(error: sqlx::Error) -> IdentityError {
    match error {
        sqlx::Error::Database(db) if db.is_unique_violation() => IdentityError::EmailTaken,
        other => IdentityError::Db(other),
    }
}

// ---------------------------------------------------------------------------------------------
// Passwords
// ---------------------------------------------------------------------------------------------

/// Verified when the account does not exist, so the response time is the same either way.
/// Computed lazily on a blocking thread, with the same parameters as real hashes.
static DUMMY_HASH: LazyLock<Option<String>> = LazyLock::new(|| {
    db::identity::hash(TemporaryPassword::generate().expose())
        .map(|hash| hash.as_str().to_owned())
        .map_err(|e| tracing::error!(error = %e, "cannot compute the dummy password hash"))
        .ok()
});

/// How many password hashes and checks run at once. Each takes about 19 MiB and a core for tens
/// of milliseconds, and the login limit is per client, so without a global bound a crowd of
/// clients could use all the memory of the server (ADR-0038). A request over the bound waits.
pub struct HashGate(tokio::sync::Semaphore);

impl HashGate {
    pub fn new(permits: usize) -> Self {
        Self(tokio::sync::Semaphore::new(permits.max(1)))
    }

    /// Runs `work` on a blocking thread once a permit is free.
    async fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, IdentityError> {
        let _permit = self.0.acquire().await.map_err(|_| IdentityError::Hashing)?;
        tokio::task::spawn_blocking(work).await.map_err(|e| {
            tracing::error!(error = %e, "password hashing task failed");
            IdentityError::Hashing
        })
    }
}

fn hashed(result: Result<HashedPassword, String>) -> Result<HashedPassword, IdentityError> {
    result.map_err(|e| {
        tracing::error!(error = %e, "password hashing failed");
        IdentityError::Hashing
    })
}

/// Hashes on a blocking thread. Call it before opening a transaction. Not bounded: for the
/// server's own CLI and for tests; a request goes through [`hash_password_gated`].
pub async fn hash_password(password: &str) -> Result<HashedPassword, IdentityError> {
    let password = password.to_owned();
    let result = tokio::task::spawn_blocking(move || db::identity::hash(&password))
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "password hashing task failed");
            IdentityError::Hashing
        })?;
    hashed(result)
}

/// Like [`hash_password`], for a request: it waits for a free permit of the gate first.
pub async fn hash_password_gated(
    gate: &HashGate,
    password: &str,
) -> Result<HashedPassword, IdentityError> {
    let password = password.to_owned();
    hashed(gate.run(move || db::identity::hash(&password)).await?)
}

/// Checks `password` against `hash` on a blocking thread, once the gate has a free permit. With no
/// hash it checks a dummy one and answers `false`, taking the same time.
pub(super) async fn verify_password(
    gate: &HashGate,
    password: String,
    hash: Option<String>,
) -> Result<bool, IdentityError> {
    gate.run(move || {
        let Ok(argon2) = db::identity::argon2() else {
            tracing::error!("argon2 parameters are invalid");
            return Err(IdentityError::Hashing);
        };
        let hash_to_check = match &hash {
            Some(hash) => Some(hash.as_str()),
            None => DUMMY_HASH.as_deref(),
        };
        let Some(hash_to_check) = hash_to_check else {
            return Ok(false);
        };
        // A malformed stored hash counts as a mismatch instead of failing the request.
        let matches = argon2
            .verify_password(password.as_bytes(), hash_to_check)
            .is_ok();
        Ok(matches && hash.is_some())
    })
    .await?
}

// ---------------------------------------------------------------------------------------------
// Per-account login throttle (ADR-0017)
// ---------------------------------------------------------------------------------------------

/// Identifies an account by the SHA-256 of its normalized email, whether or not it exists, so
/// the throttle neither leaks which accounts exist nor stores addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThrottleKey([u8; 32]);

impl ThrottleKey {
    pub fn new(email: &str) -> Self {
        Self(Sha256::digest(email.trim().to_lowercase().as_bytes()).into())
    }
}

struct Attempts {
    count: u32,
    window_start: Instant,
}

/// Counts login attempts per account in memory: this is a single binary, so one map is enough.
/// An attempt is counted when it starts, not when it fails, so a burst of parallel guesses is
/// stopped after `max_attempts` of them instead of after the slow hash of each (ADR-0038). A
/// success resets the count. A restart clears the map, and an attacker can lock a known email
/// out for one window.
pub struct LoginThrottle {
    max_attempts: u32,
    window: Duration,
    capacity: usize,
    attempts: Mutex<HashMap<ThrottleKey, Attempts>>,
}

impl Default for LoginThrottle {
    fn default() -> Self {
        Self::with_limits(LOGIN_MAX_ATTEMPTS, LOGIN_WINDOW, THROTTLE_CAPACITY)
    }
}

impl LoginThrottle {
    pub fn with_limits(max_attempts: u32, window: Duration, capacity: usize) -> Self {
        Self {
            max_attempts,
            window,
            capacity,
            attempts: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ThrottleKey, Attempts>> {
        // The map holds plain counters, so a panic elsewhere cannot leave it inconsistent.
        self.attempts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Counts one attempt on the account, or refuses it when the window's attempts are used up.
    pub fn reserve(&self, key: &ThrottleKey) -> Result<(), IdentityError> {
        let mut map = self.lock();
        if let Some(entry) = map.get_mut(key) {
            let elapsed = entry.window_start.elapsed();
            if elapsed >= self.window {
                *entry = Attempts {
                    count: 1,
                    window_start: Instant::now(),
                };
            } else if entry.count >= self.max_attempts {
                return Err(IdentityError::TooManyAttempts {
                    retry_after_secs: self.window.saturating_sub(elapsed).as_secs().max(1),
                });
            } else {
                entry.count += 1;
            }
            return Ok(());
        }
        if map.len() >= self.capacity {
            self.make_room(&mut map);
        }
        map.insert(
            *key,
            Attempts {
                count: 1,
                window_start: Instant::now(),
            },
        );
        Ok(())
    }

    pub fn reset(&self, key: &ThrottleKey) {
        self.lock().remove(key);
    }

    /// Drops expired entries and, if the map is still full, the oldest tenth, accounts that are
    /// not locked out first, so spraying random emails cannot grow memory, cannot free a locked
    /// account while others can go, and a full map is not rescanned on every insert.
    fn make_room(&self, map: &mut HashMap<ThrottleKey, Attempts>) {
        map.retain(|_, a| a.window_start.elapsed() < self.window);
        if map.len() < self.capacity {
            return;
        }
        let max = self.max_attempts;
        let mut by_age: Vec<(bool, Instant, ThrottleKey)> = map
            .iter()
            .map(|(k, a)| (a.count >= max, a.window_start, *k))
            .collect();
        by_age.sort_unstable_by_key(|(locked, started, _)| (*locked, *started));
        for (_, _, key) in by_age.into_iter().take((self.capacity / 10).max(1)) {
            map.remove(&key);
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().len()
    }
}

// ---------------------------------------------------------------------------------------------
// Users
// ---------------------------------------------------------------------------------------------

/// Creates a user with a password identity and `must_change_password = true` (ADR-0017: the
/// password is a generated temporary one). The low-level function: it audits nothing, so it is
/// private. The admin API goes through [`admin_create_user`] and the server CLI through
/// [`create_user_from_cli`], which do (ADR-0034).
async fn create_user(
    conn: &mut PgConnection,
    new: NewUser,
    password: HashedPassword,
) -> Result<User, IdentityError> {
    let user = repo::insert_user(conn, Uuid::now_v7(), &new, true)
        .await
        .map_err(email_taken)?;
    repo::insert_password_identity(conn, user.id, password.as_str()).await?;
    Ok(user)
}

/// Seeds a user for the unit tests of any module, with no audit row. Not compiled into the
/// server: production code creates users only through the audited functions (ADR-0034).
#[cfg(test)]
pub async fn create_user_unaudited_for_tests(
    conn: &mut PgConnection,
    new: NewUser,
    password: HashedPassword,
) -> Result<User, IdentityError> {
    create_user(conn, new, password).await
}

/// Creates a user for the server CLI, where nobody is signed in, and records it in the audit log
/// with no actor and the source `cli` (ADR-0034).
pub async fn create_user_from_cli(
    conn: &mut PgConnection,
    new: NewUser,
    password: HashedPassword,
) -> Result<User, IdentityError> {
    let role = new.role;
    let user = create_user(conn, new, password).await?;
    events::audit_cli(
        conn,
        user_entry("user.create", user.id, json!({"role": role.as_str()})),
    )
    .await?;
    Ok(user)
}

/// An audit entry about a user. `meta` holds ids, roles and field names only (ADR-0034): never a
/// password, a hash, a token, an email address or a name.
fn user_entry(action: &'static str, user_id: Uuid, meta: serde_json::Value) -> AuditEntry {
    AuditEntry {
        action,
        resource_type: "user",
        resource_id: user_id,
        meta,
    }
}

// ---------------------------------------------------------------------------------------------
// Login and sessions
// ---------------------------------------------------------------------------------------------

/// A login in progress: the account lookup is done, the password is not checked yet.
pub struct LoginAttempt {
    key: ThrottleKey,
    record: Option<LoginRecord>,
}

/// A login whose password was right: the user and the exact hash that was verified. A session is
/// created only against that hash (see [`create_session`]).
pub struct VerifiedLogin {
    pub user: User,
    password_hash: String,
}

/// First half of a login: counts the attempt on the account and refuses it when the window's
/// attempts are used up, before any hashing, then loads the user.
pub async fn lookup_login(
    conn: &mut PgConnection,
    throttle: &LoginThrottle,
    email: &str,
) -> Result<LoginAttempt, IdentityError> {
    let key = ThrottleKey::new(email);
    throttle.reserve(&key)?;
    let record = match super::model::Email::parse(email) {
        Ok(email) => repo::find_login(conn, email.as_str()).await?,
        Err(_) => None,
    };
    Ok(LoginAttempt { key, record })
}

/// Second half of a login, with no database access: verifies the password (a dummy hash when
/// there is no account) and resets the account's count on a success. Every failure is the same
/// error.
pub async fn complete_login(
    throttle: &LoginThrottle,
    gate: &HashGate,
    attempt: LoginAttempt,
    password: &str,
) -> Result<VerifiedLogin, IdentityError> {
    let LoginAttempt { key, record } = attempt;
    let hash = record.as_ref().map(|r| r.password_hash.clone());
    let verified = verify_password(gate, password.to_owned(), hash).await?;
    match record {
        Some(record) if verified && record.user.status == UserStatus::Active => {
            throttle.reset(&key);
            if !record.email_verified {
                return Err(IdentityError::EmailNotVerified);
            }
            Ok(VerifiedLogin {
                user: record.user,
                password_hash: record.password_hash,
            })
        }
        _ => Err(IdentityError::InvalidCredentials),
    }
}

/// Creates the session of a verified login. In this transaction the user and the password
/// identity are locked `FOR SHARE` and read again: the account must still be active and the hash
/// must still be the one that was verified. A password reset, a password change or a disabling that
/// ran while the hash was being checked therefore either waits for this commit and deletes the
/// session, or has committed already and refuses the login (ADR-0038).
pub async fn create_session(
    conn: &mut PgConnection,
    login: &VerifiedLogin,
    kind: SessionKind,
) -> Result<NewSession, IdentityError> {
    if !repo::lock_verified_login(conn, login.user.id, &login.password_hash).await? {
        return Err(IdentityError::InvalidCredentials);
    }
    let token = SessionToken::generate();
    let expires_at = repo::insert_session(
        conn,
        login.user.id,
        &token.hash(),
        kind,
        kind.initial_lifetime().num_seconds(),
    )
    .await?;
    Ok(NewSession {
        token,
        kind,
        expires_at,
    })
}

/// A request's session, resolved.
pub struct Authenticated {
    pub actor: Actor,
    pub must_change_password: bool,
    /// Set when the session should be refreshed (not seen for an hour); see [`touch_session`].
    pub touch: Option<SessionKind>,
}

/// Resolves a token to its actor. A missing, expired or disabled-user session is `Unauthenticated`.
pub async fn authenticate(
    conn: &mut PgConnection,
    token: &SessionToken,
) -> Result<Authenticated, IdentityError> {
    let found = repo::find_session(conn, &token.hash())
        .await?
        .ok_or(IdentityError::Unauthenticated)?;
    Ok(Authenticated {
        actor: Actor::new(found.user_id, found.role, found.session_id),
        must_change_password: found.must_change_password,
        touch: found.stale.then_some(found.kind),
    })
}

/// Refreshes `last_seen_at` and, for web and app sessions, the expiry. A no-op unless the
/// session has not been refreshed for an hour.
pub async fn touch_session(
    conn: &mut PgConnection,
    session_id: Uuid,
    kind: SessionKind,
) -> Result<(), IdentityError> {
    let slide = kind.sliding_lifetime().map(|d| d.num_seconds());
    repo::touch_session(conn, session_id, slide).await?;
    Ok(())
}

/// Deletes the actor's current session.
pub async fn logout(conn: &mut PgConnection, actor: &Actor) -> Result<(), IdentityError> {
    policy::authorize(actor, Action::ManageOwnAccount)?;
    repo::delete_session(conn, actor.session_id(), actor.user_id()).await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Registration, verification and password reset (ADR-0046)
// ---------------------------------------------------------------------------------------------

/// A registration that passed every check and is hashed, ready for [`register`].
pub struct PreparedRegistration {
    new: NewUser,
    password: HashedPassword,
}

/// What a registration is checked against: the configuration, not the request.
pub struct RegistrationRules<'a> {
    pub allowed_domains: &'a [String],
    pub password_min_length: usize,
    pub institution_name: &'a str,
}

/// The slow, database-free half of a registration: checks the address's domain, the names and the
/// password policy, then hashes. It always hashes, so the time does not depend on whether the
/// address has an account.
pub async fn prepare_registration(
    gate: &HashGate,
    rules: RegistrationRules<'_>,
    name: &str,
    surname: &str,
    email: &str,
    password: String,
) -> Result<PreparedRegistration, IdentityError> {
    let email = Email::parse(email)?;
    let name = PersonName::name(name)?;
    let surname = PersonName::surname(surname)?;
    let domain = email.as_str().rsplit_once('@').map_or("", |(_, d)| d);
    if !rules
        .allowed_domains
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(domain))
    {
        return Err(IdentityError::EmailDomainNotAllowed);
    }
    let policy = PasswordPolicy::new(
        rules.password_min_length,
        email.as_str(),
        name.as_str(),
        surname.as_str(),
        rules.institution_name,
    );
    let password = Password::parse(password, &policy)?;
    let password = hash_password_gated(gate, password.expose()).await?;
    Ok(PreparedRegistration {
        new: NewUser {
            email,
            name,
            surname,
            role: Role::Student,
        },
        password,
    })
}

/// Creates the unverified account, or, for an unverified active student, replaces what the earlier
/// registration chose; any other account is left alone. Emits the event that sends the link in all
/// the first two cases and does nothing else, so the caller answers the same either way.
pub async fn register(
    conn: &mut PgConnection,
    prepared: PreparedRegistration,
) -> Result<(), IdentityError> {
    let PreparedRegistration { new, password } = prepared;
    let id = Uuid::now_v7();
    if let Some(user_id) = repo::insert_registered_user(conn, id, &new).await? {
        repo::insert_password_identity(conn, user_id, password.as_str()).await?;
        events::emit(conn, Event::EmailVerificationRequested { user_id }).await?;
        return Ok(());
    }
    let Some(account) = repo::lock_account_by_email(conn, new.email.as_str()).await? else {
        return Ok(());
    };
    if account.status == UserStatus::Active
        && account.role == Role::Student
        && !account.email_verified
    {
        repo::update_names(conn, account.id, new.name.as_str(), new.surname.as_str()).await?;
        repo::upsert_password_hash(conn, account.id, password.as_str()).await?;
        repo::void_email_tokens(conn, account.id, TokenPurpose::Verify).await?;
        events::emit(
            conn,
            Event::EmailVerificationRequested {
                user_id: account.id,
            },
        )
        .await?;
    }
    Ok(())
}

/// Uses the verification token up and marks its address verified.
pub async fn verify_email(
    conn: &mut PgConnection,
    token: Option<EmailToken>,
) -> Result<(), IdentityError> {
    let token = token.ok_or(IdentityError::TokenInvalid)?;
    let user_id = repo::consume_email_token(conn, &token.hash(), TokenPurpose::Verify)
        .await?
        .ok_or(IdentityError::TokenInvalid)?;
    repo::mark_email_verified(conn, user_id).await?;
    Ok(())
}

/// Emits the event that sends a reset link, for an active account. An unknown, malformed or
/// disabled address does nothing and is not an error: the caller answers the same (ADR-0046).
pub async fn request_password_reset(
    conn: &mut PgConnection,
    email: &str,
) -> Result<(), IdentityError> {
    let Ok(email) = Email::parse(email) else {
        return Ok(());
    };
    if let Some(account) = repo::find_account_by_email(conn, email.as_str()).await?
        && account.status == UserStatus::Active
    {
        events::emit(
            conn,
            Event::PasswordResetRequested {
                user_id: account.id,
            },
        )
        .await?;
    }
    Ok(())
}

/// Whose reset it is: what the password policy needs as context.
pub struct ResetTarget {
    email: String,
    name: String,
    surname: String,
}

/// Looks the reset token up without using it up. A missing, spent or expired token, or one of a
/// disabled account, is [`IdentityError::TokenInvalid`], and that is decided before the password is
/// looked at.
pub async fn check_reset_token(
    conn: &mut PgConnection,
    token: Option<&EmailToken>,
) -> Result<ResetTarget, IdentityError> {
    let token = token.ok_or(IdentityError::TokenInvalid)?;
    let account = repo::find_token_account(conn, &token.hash(), TokenPurpose::Reset)
        .await?
        .filter(|account| account.status == UserStatus::Active)
        .ok_or(IdentityError::TokenInvalid)?;
    Ok(ResetTarget {
        email: account.email,
        name: account.name,
        surname: account.surname,
    })
}

/// A new password that passed the policy and is hashed, ready for [`confirm_password_reset`].
pub struct PreparedReset {
    password: HashedPassword,
}

pub async fn prepare_password_reset(
    gate: &HashGate,
    target: ResetTarget,
    new_password: String,
    min_len: usize,
    institution: &str,
) -> Result<PreparedReset, IdentityError> {
    let policy = PasswordPolicy::new(
        min_len,
        &target.email,
        &target.name,
        &target.surname,
        institution,
    );
    let password = Password::parse(new_password, &policy)?;
    let password = hash_password_gated(gate, password.expose()).await?;
    Ok(PreparedReset { password })
}

/// Uses the token up, sets the password and revokes every session of the user. The link proved
/// the mailbox, so the address counts as verified too.
pub async fn confirm_password_reset(
    conn: &mut PgConnection,
    token: Option<&EmailToken>,
    prepared: PreparedReset,
) -> Result<(), IdentityError> {
    let token = token.ok_or(IdentityError::TokenInvalid)?;
    let user_id = repo::consume_email_token(conn, &token.hash(), TokenPurpose::Reset)
        .await?
        .ok_or(IdentityError::TokenInvalid)?;
    let account = repo::find_user_shared(conn, user_id)
        .await?
        .filter(|user| user.status == UserStatus::Active)
        .ok_or(IdentityError::TokenInvalid)?;
    repo::upsert_password_hash(conn, account.id, prepared.password.as_str()).await?;
    repo::mark_email_verified(conn, account.id).await?;
    repo::set_must_change_password(conn, account.id, false).await?;
    repo::void_email_tokens(conn, account.id, TokenPurpose::Reset).await?;
    repo::delete_sessions_of(conn, account.id).await?;
    Ok(())
}

/// Who a link is for and the token that is in it. The token exists only here and in the mail.
pub struct MintedToken {
    pub token: EmailToken,
    pub email: String,
    pub name: String,
    pub surname: String,
}

/// The first half of sending a link, run by the outbox handler (ADR-0045, ADR-0046): voids the
/// user's earlier unused tokens of this purpose and stores the hash of a new one. `None` when the
/// event no longer applies (the address is verified already, the account is disabled or gone).
pub async fn mint_email_token(
    conn: &mut PgConnection,
    user_id: Uuid,
    purpose: TokenPurpose,
) -> Result<Option<MintedToken>, IdentityError> {
    let Some(account) = repo::find_account(conn, user_id).await? else {
        return Ok(None);
    };
    let applies = account.status == UserStatus::Active
        && (purpose == TokenPurpose::Reset || !account.email_verified);
    if !applies {
        return Ok(None);
    }
    repo::void_email_tokens(conn, user_id, purpose).await?;
    let token = EmailToken::generate();
    repo::insert_email_token(
        conn,
        user_id,
        purpose,
        &token.hash(),
        purpose.lifetime().num_seconds(),
    )
    .await?;
    Ok(Some(MintedToken {
        token,
        email: account.email,
        name: account.name,
        surname: account.surname,
    }))
}

// ---------------------------------------------------------------------------------------------
// Sign-in with Google (ADR-0047)
// ---------------------------------------------------------------------------------------------

/// How long a started Google sign-in can be finished.
const GOOGLE_ATTEMPT_LIFETIME_SECS: i64 = 10 * 60;

/// Remembers a started sign-in and returns the value to set as the browser's cookie.
pub async fn start_google_sign_in(
    conn: &mut PgConnection,
    begin: &Begin,
    app_challenge: Option<&str>,
) -> Result<Option<BrowserBinding>, IdentityError> {
    // An app holds the secret, so its attempt is tied to that; a browser's is tied to a cookie (ADR-0062).
    let (binding, challenge) = match app_challenge {
        Some(hex) => (
            None,
            Some(parse_hex_32(hex).ok_or(IdentityError::AppChallengeInvalid)?),
        ),
        None => (Some(BrowserBinding::generate()), None),
    };
    repo::insert_oauth_state(
        conn,
        &Sha256::digest(begin.state.as_bytes()),
        binding
            .as_ref()
            .map(|b| b.hash())
            .as_ref()
            .map(<[u8; 32]>::as_slice),
        challenge.as_ref().map(<[u8; 32]>::as_slice),
        &begin.nonce,
        &begin.pkce_verifier,
        GOOGLE_ATTEMPT_LIFETIME_SECS,
    )
    .await?;
    Ok(binding)
}

/// 64 lowercase hex characters as 32 bytes.
fn parse_hex_32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let mut out = [0_u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// What a started sign-in remembered: the nonce the ID token must carry and the PKCE verifier, and the
/// challenge of the app that started it, if one did.
pub struct GoogleAttempt {
    pub nonce: String,
    pub pkce_verifier: String,
    pub app_challenge: Option<[u8; 32]>,
}

/// Uses the attempt up. It exists once, for ten minutes, and for the browser that started it, or for the
/// app: an app's attempt is not tied to a browser.
pub async fn take_google_attempt(
    conn: &mut PgConnection,
    state: &str,
    binding: Option<&BrowserBinding>,
) -> Result<GoogleAttempt, IdentityError> {
    let taken = repo::take_oauth_state(
        conn,
        &Sha256::digest(state.as_bytes()),
        binding.map(|b| b.hash()).as_ref().map(<[u8; 32]>::as_slice),
    )
    .await?
    .ok_or(IdentityError::GoogleAttemptInvalid)?;
    Ok(GoogleAttempt {
        nonce: taken.nonce,
        pkce_verifier: taken.pkce_verifier,
        app_challenge: taken.app_challenge.and_then(|c| c.try_into().ok()),
    })
}

/// How long the app has to pick up the code: it is delivered by a link, a moment after the sign-in.
const APP_HANDOFF_LIFETIME_SECS: i64 = 60;

/// The code that ends an app's sign-in with Google. Only its SHA-256 is stored.
pub async fn create_app_handoff(
    conn: &mut PgConnection,
    user: &User,
    app_challenge: &[u8; 32],
) -> Result<String, IdentityError> {
    if !repo::lock_active_user(conn, user.id).await? {
        return Err(IdentityError::AccountDisabled);
    }
    let code = db::identity::random_base64url::<32>();
    repo::insert_app_handoff(
        conn,
        &Sha256::digest(code.as_bytes()),
        user.id,
        app_challenge,
        APP_HANDOFF_LIFETIME_SECS,
    )
    .await?;
    Ok(code)
}

/// Trades a handoff and the secret behind its challenge for an app session. The code is used up by the
/// attempt, right or wrong.
pub async fn exchange_app_handoff(
    conn: &mut PgConnection,
    code: &str,
    verifier: &str,
) -> Result<(User, NewSession), IdentityError> {
    let (user_id, challenge) = repo::take_app_handoff(conn, &Sha256::digest(code.as_bytes()))
        .await?
        .ok_or(IdentityError::GoogleAttemptInvalid)?;
    if Sha256::digest(verifier.as_bytes()).as_slice() != challenge.as_slice() {
        return Err(IdentityError::GoogleAttemptInvalid);
    }
    let user = repo::find_user(conn, user_id)
        .await?
        .ok_or(IdentityError::GoogleAttemptInvalid)?;
    let session = create_session_for(conn, &user, SessionKind::App).await?;
    Ok((user, session))
}

/// A name for the account from what Google sent, cut to the limit; `fallback` when there is none.
fn google_name(
    given: Option<&str>,
    fallback: &str,
    surname: bool,
) -> Result<PersonName, ModelError> {
    let text = given
        .map(str::trim)
        .filter(|text| !text.is_empty() && !text.chars().any(char::is_control))
        .unwrap_or(fallback);
    let cut: String = text.chars().take(100).collect();
    if surname {
        PersonName::surname(&cut)
    } else {
        PersonName::name(&cut)
    }
}

/// Finds, links or creates the account of a verified Google profile (ADR-0047, point 5). Returns
/// the user to open a session for.
pub async fn sign_in_with_google(
    conn: &mut PgConnection,
    profile: &GoogleProfile,
    allowed_domains: &[String],
) -> Result<User, IdentityError> {
    let email = Email::parse(&profile.email).map_err(|_| IdentityError::GoogleAccountNotAllowed)?;
    let domain = email.as_str().rsplit_once('@').map_or("", |(_, d)| d);
    let allowed = allowed_domains
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(domain));
    if !profile.email_verified || !allowed {
        return Err(IdentityError::GoogleAccountNotAllowed);
    }

    let user_id = match repo::find_google_user(conn, &profile.subject).await? {
        Some(user_id) => user_id,
        None => match repo::lock_account_by_email(conn, email.as_str()).await? {
            Some(account) => {
                repo::insert_google_identity(conn, account.id, &profile.subject).await?;
                if !account.email_verified {
                    // Whoever pre-registered this address chose a password for it. Google has just
                    // shown that the address belongs to someone else, so that password goes.
                    repo::delete_password_identity(conn, account.id).await?;
                    repo::void_email_tokens(conn, account.id, TokenPurpose::Verify).await?;
                    repo::delete_sessions_of(conn, account.id).await?;
                    repo::mark_email_verified(conn, account.id).await?;
                }
                account.id
            }
            None => {
                let local = email.as_str().split('@').next().unwrap_or_default();
                let new = NewUser {
                    name: google_name(profile.given_name.as_deref(), local, false)?,
                    surname: google_name(profile.family_name.as_deref(), "Student", true)?,
                    email,
                    role: Role::Student,
                };
                let user = repo::insert_user(conn, Uuid::now_v7(), &new, false)
                    .await
                    .map_err(email_taken)?;
                repo::insert_google_identity(conn, user.id, &profile.subject).await?;
                user.id
            }
        },
    };
    let user = repo::find_user_shared(conn, user_id)
        .await?
        .ok_or(IdentityError::UserNotFound)?;
    if user.status != UserStatus::Active {
        return Err(IdentityError::AccountDisabled);
    }
    Ok(user)
}

/// A session for a user who has just proved themselves some other way than with a password. The
/// user is locked and checked again, like in [`create_session`].
pub async fn create_session_for(
    conn: &mut PgConnection,
    user: &User,
    kind: SessionKind,
) -> Result<NewSession, IdentityError> {
    if !repo::lock_active_user(conn, user.id).await? {
        return Err(IdentityError::AccountDisabled);
    }
    let token = SessionToken::generate();
    let expires_at = repo::insert_session(
        conn,
        user.id,
        &token.hash(),
        kind,
        kind.initial_lifetime().num_seconds(),
    )
    .await?;
    Ok(NewSession {
        token,
        kind,
        expires_at,
    })
}

// ---------------------------------------------------------------------------------------------
// Admin: users
// ---------------------------------------------------------------------------------------------

pub async fn list_users(
    conn: &mut PgConnection,
    actor: &Actor,
    mut filter: UserFilter,
    page: PageParams,
) -> Result<Page<User>, IdentityError> {
    policy::authorize(actor, Action::ManageUsers)?;
    filter.email_contains = filter.email_contains.map(|text| text.to_lowercase());
    let rows = repo::list_users(conn, &filter, page.after, i64::from(page.limit) + 1).await?;
    Ok(Page::from_rows(rows, page.limit, |u| u.id))
}

/// People by role, and how many are disabled. Admins only (ADR-0053).
pub async fn user_summary(
    conn: &mut PgConnection,
    actor: &Actor,
) -> Result<RoleCounts, IdentityError> {
    policy::authorize(actor, Action::ViewAdminReports)?;
    Ok(repo::role_counts(conn).await?)
}

/// What one person does in the institution: groups, courses, last sign-in. Admins only
/// (ADR-0053).
pub async fn user_facts(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<UserFacts, IdentityError> {
    policy::authorize(actor, Action::ViewAdminReports)?;
    let user = repo::find_user(conn, id)
        .await?
        .ok_or(IdentityError::UserNotFound)?;
    let last_login_at = repo::last_login_at(conn, id).await?;
    let groups = org::groups_of_user(conn, id)
        .await
        .map_err(|e| IdentityError::Org(Box::new(e)))?;
    let courses = teaching::user_course_facts(conn, id)
        .await
        .map_err(|e| IdentityError::Teaching(Box::new(e)))?;
    Ok(UserFacts {
        user,
        last_login_at,
        groups,
        courses,
    })
}

/// Creates a user with a temporary password for an admin. The hash is computed by the caller
/// before the transaction starts.
pub async fn admin_create_user(
    conn: &mut PgConnection,
    actor: &Actor,
    new: NewUser,
    temporary_password: HashedPassword,
) -> Result<User, IdentityError> {
    policy::authorize(actor, Action::ManageUsers)?;
    let role = new.role;
    let user = create_user(conn, new, temporary_password).await?;
    events::audit(
        conn,
        actor,
        user_entry("user.create", user.id, json!({"role": role.as_str()})),
    )
    .await?;
    Ok(user)
}

/// Changes a user's name, role or status. Disabling also deletes all their sessions, in the
/// same transaction. The last active admin can be neither demoted nor disabled.
pub async fn update_user(
    conn: &mut PgConnection,
    actor: &Actor,
    user_id: Uuid,
    update: UserUpdate,
) -> Result<User, IdentityError> {
    policy::authorize(actor, Action::ManageUsers)?;
    let active_admins = repo::lock_active_admins(conn).await?;
    let target = repo::find_user(conn, user_id)
        .await?
        .ok_or(IdentityError::UserNotFound)?;
    let stops_being_active_admin = update.role.is_some_and(|role| role != Role::Admin)
        || update.status == Some(UserStatus::Disabled);
    if stops_being_active_admin && active_admins == [user_id] {
        return Err(IdentityError::LastAdmin);
    }
    let updated = repo::update_user(
        conn,
        target.id,
        update.name.as_ref().map(PersonName::as_str),
        update.surname.as_ref().map(PersonName::as_str),
        update.role,
        update.status,
    )
    .await?
    .ok_or(IdentityError::UserNotFound)?;
    if update.status == Some(UserStatus::Disabled) {
        repo::delete_sessions_of(conn, updated.id).await?;
    }
    // A teacher or admin who becomes a student: they must not own a course, and they stop being a
    // course teacher everywhere (ADR-0037). This comes after the UPDATE, whose row lock waits for
    // a course being created for this user, so the check sees it; a refusal rolls the UPDATE back.
    let demoted =
        updated.role == Role::Student && matches!(target.role, Role::Teacher | Role::Admin);
    if demoted {
        if teaching::owns_courses(conn, updated.id)
            .await
            .map_err(|e| IdentityError::Teaching(Box::new(e)))?
        {
            return Err(IdentityError::OwnsCourses);
        }
        teaching::drop_teacher_memberships(conn, actor, updated.id)
            .await
            .map_err(|e| IdentityError::Teaching(Box::new(e)))?;
    }
    // One row for each kind of change, and none for a field that was sent with its old value.
    let mut renamed = Vec::new();
    if update
        .name
        .as_ref()
        .is_some_and(|n| n.as_str() != target.name)
    {
        renamed.push("name");
    }
    if update
        .surname
        .as_ref()
        .is_some_and(|n| n.as_str() != target.surname)
    {
        renamed.push("surname");
    }
    if !renamed.is_empty() {
        events::audit(
            conn,
            actor,
            user_entry("user.rename", target.id, json!({"fields": renamed})),
        )
        .await?;
    }
    if let Some(to) = update.role.filter(|role| *role != target.role) {
        let meta = json!({"from": target.role.as_str(), "to": to.as_str()});
        events::audit(conn, actor, user_entry("user.role_change", target.id, meta)).await?;
    }
    if let Some(status) = update.status.filter(|status| *status != target.status) {
        let action = match status {
            UserStatus::Disabled => "user.disable",
            UserStatus::Active => "user.enable",
        };
        events::audit(conn, actor, user_entry(action, target.id, json!({}))).await?;
    }
    Ok(updated)
}

/// Replaces a user's password with a temporary one they must change, and signs them out
/// everywhere.
pub async fn reset_password(
    conn: &mut PgConnection,
    actor: &Actor,
    user_id: Uuid,
    temporary_password: HashedPassword,
) -> Result<(), IdentityError> {
    policy::authorize(actor, Action::ManageUsers)?;
    repo::find_user(conn, user_id)
        .await?
        .ok_or(IdentityError::UserNotFound)?;
    repo::upsert_password_hash(conn, user_id, temporary_password.as_str()).await?;
    repo::set_must_change_password(conn, user_id, true).await?;
    repo::delete_sessions_of(conn, user_id).await?;
    events::audit(
        conn,
        actor,
        user_entry("user.password_reset", user_id, json!({})),
    )
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Own account
// ---------------------------------------------------------------------------------------------

/// A user by id, for other modules that have already authorized their own action (for example
/// to check the institution role of someone about to join a course). Callers must have
/// authorized the action already: this checks nothing.
pub async fn find_user(conn: &mut PgConnection, id: Uuid) -> Result<Option<User>, IdentityError> {
    Ok(repo::find_user(conn, id).await?)
}

/// The users with these ids, in no particular order; an id that names no user is absent. Like
/// [`find_user`], it checks nothing: the caller has authorized its own action.
/// Like [`find_user`], with the row locked `FOR SHARE` until the transaction ends, so a role
/// change or a disabling of that user cannot slip in between the check and the write that relies on
/// it (ADR-0037).
pub async fn find_user_shared(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<User>, IdentityError> {
    Ok(repo::find_user_shared(conn, id).await?)
}

pub async fn find_users(conn: &mut PgConnection, ids: &[Uuid]) -> Result<Vec<User>, IdentityError> {
    Ok(repo::find_users(conn, ids).await?)
}

pub async fn get_me(conn: &mut PgConnection, actor: &Actor) -> Result<User, IdentityError> {
    policy::authorize(actor, Action::ManageOwnAccount)?;
    repo::find_user(conn, actor.user_id())
        .await?
        .ok_or(IdentityError::UserNotFound)
}

/// What a password change needs from the database, read before any hashing starts.
pub struct PasswordChangeContext {
    user: User,
    old_hash: String,
}

pub async fn password_change_context(
    conn: &mut PgConnection,
    actor: &Actor,
) -> Result<PasswordChangeContext, IdentityError> {
    policy::authorize(actor, Action::ManageOwnAccount)?;
    let user = repo::find_user(conn, actor.user_id())
        .await?
        .ok_or(IdentityError::UserNotFound)?;
    // An account without a password identity has no current password to confirm.
    let old_hash = repo::find_password_hash(conn, user.id)
        .await?
        .ok_or(IdentityError::WrongCurrentPassword)?;
    Ok(PasswordChangeContext { user, old_hash })
}

/// A new password that passed the policy and is already hashed, ready for [`change_password`].
pub struct PreparedPasswordChange {
    old_hash: String,
    new_hash: HashedPassword,
}

/// Does all the slow, database-free work of a password change: confirms the current password
/// (counting every attempt against the account's login throttle, since a stolen session could
/// otherwise guess it), applies the policy and hashes the new one.
pub async fn prepare_password_change(
    throttle: &LoginThrottle,
    gate: &HashGate,
    context: PasswordChangeContext,
    current: &str,
    new: String,
    min_len: usize,
    institution: &str,
) -> Result<PreparedPasswordChange, IdentityError> {
    let PasswordChangeContext { user, old_hash } = context;
    let key = ThrottleKey::new(&user.email);
    throttle.reserve(&key)?;
    if !verify_password(gate, current.to_owned(), Some(old_hash.clone())).await? {
        return Err(IdentityError::WrongCurrentPassword);
    }
    throttle.reset(&key);
    if new == current {
        return Err(IdentityError::PasswordUnchanged);
    }
    let policy = PasswordPolicy::new(min_len, &user.email, &user.name, &user.surname, institution);
    let password = Password::parse(new, &policy)?;
    let new_hash = hash_password_gated(gate, password.expose()).await?;
    Ok(PreparedPasswordChange { old_hash, new_hash })
}

/// Stores the new password, clears the forced-change flag and revokes every other session.
pub async fn change_password(
    conn: &mut PgConnection,
    actor: &Actor,
    prepared: PreparedPasswordChange,
) -> Result<(), IdentityError> {
    policy::authorize(actor, Action::ManageOwnAccount)?;
    let changed = repo::swap_password_hash(
        conn,
        actor.user_id(),
        &prepared.old_hash,
        prepared.new_hash.as_str(),
    )
    .await?;
    if changed == 0 {
        return Err(IdentityError::ConcurrentChange);
    }
    repo::set_must_change_password(conn, actor.user_id(), false).await?;
    repo::delete_other_sessions(conn, actor.user_id(), actor.session_id()).await?;
    Ok(())
}

pub async fn list_sessions(
    conn: &mut PgConnection,
    actor: &Actor,
    page: PageParams,
) -> Result<Page<SessionInfo>, IdentityError> {
    policy::authorize(actor, Action::ManageOwnAccount)?;
    let rows =
        repo::list_sessions(conn, actor.user_id(), page.after, i64::from(page.limit) + 1).await?;
    Ok(Page::from_rows(rows, page.limit, |s| s.id))
}

/// Revokes one of the actor's own sessions. Someone else's session looks like a missing one.
pub async fn revoke_session(
    conn: &mut PgConnection,
    actor: &Actor,
    session_id: Uuid,
) -> Result<(), IdentityError> {
    policy::authorize(actor, Action::ManageOwnAccount)?;
    match repo::delete_session(conn, session_id, actor.user_id()).await? {
        0 => Err(IdentityError::SessionNotFound),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests;
