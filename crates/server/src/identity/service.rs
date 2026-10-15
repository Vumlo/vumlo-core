//! Business rules. Every function that acts for a user authorizes first, takes the connection
//! it runs on and never begins or commits a transaction. Password hashing is exposed separately
//! so callers can do it before they open one.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::{Algorithm, Argon2, Params, Version};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{
    HashedPassword, ModelError, NewSession, NewUser, Page, PageParams, Password, PasswordPolicy,
    SessionInfo, SessionKind, SessionToken, TemporaryPassword, User, UserStatus,
};
use super::repo::{self, LoginRecord};
use crate::actor::Actor;
use crate::policy::{self, Action, PolicyError};

/// OWASP's Argon2id minimum: 19 MiB, 2 iterations, 1 lane (ADR-0017).
const ARGON2_MEMORY_KIB: u32 = 19 * 1024;
const ARGON2_ITERATIONS: u32 = 2;
const ARGON2_LANES: u32 = 1;

const LOGIN_MAX_FAILURES: u32 = 10;
const LOGIN_WINDOW: Duration = Duration::from_secs(15 * 60);
const THROTTLE_CAPACITY: usize = 100_000;

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
    #[error("the email is already in use")]
    EmailTaken,
    #[error("the account changed while the request was running")]
    ConcurrentChange,
    #[error("user not found")]
    UserNotFound,
    #[error("session not found")]
    SessionNotFound,
    #[error("password hashing failed")]
    Hashing,
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

static ARGON2: LazyLock<Result<Argon2<'static>, argon2::Error>> = LazyLock::new(|| {
    let params = Params::new(ARGON2_MEMORY_KIB, ARGON2_ITERATIONS, ARGON2_LANES, None)?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
});

/// Verified when the account does not exist, so the response time is the same either way.
/// Computed lazily on a blocking thread, with the same parameters as real hashes.
static DUMMY_HASH: LazyLock<Option<String>> = LazyLock::new(|| {
    hash_blocking(TemporaryPassword::generate().expose())
        .map_err(|e| tracing::error!(error = %e, "cannot compute the dummy password hash"))
        .ok()
});

fn hash_blocking(password: &str) -> Result<String, String> {
    let argon2 = ARGON2.as_ref().map_err(|e| e.to_string())?;
    argon2
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|e| e.to_string())
}

/// Hashes on a blocking thread. Call it before opening a transaction.
pub async fn hash_password(password: &str) -> Result<HashedPassword, IdentityError> {
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || hash_blocking(&password))
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "password hashing task failed");
            IdentityError::Hashing
        })?
        .map(HashedPassword::new)
        .map_err(|e| {
            tracing::error!(error = %e, "password hashing failed");
            IdentityError::Hashing
        })
}

/// Checks `password` against `hash` on a blocking thread. With no hash it checks a dummy one
/// and answers `false`, taking the same time.
pub(super) async fn verify_password(
    password: String,
    hash: Option<String>,
) -> Result<bool, IdentityError> {
    tokio::task::spawn_blocking(move || {
        let Ok(argon2) = ARGON2.as_ref() else {
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
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "password verification task failed");
        IdentityError::Hashing
    })?
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

struct Failures {
    count: u32,
    window_start: Instant,
}

/// Counts failed logins per account in memory: this is a single binary, so one map is enough.
/// A restart clears it, and an attacker can lock a known email out for one window.
pub struct LoginThrottle {
    max_failures: u32,
    window: Duration,
    capacity: usize,
    failures: Mutex<HashMap<ThrottleKey, Failures>>,
}

impl Default for LoginThrottle {
    fn default() -> Self {
        Self::with_limits(LOGIN_MAX_FAILURES, LOGIN_WINDOW, THROTTLE_CAPACITY)
    }
}

impl LoginThrottle {
    pub fn with_limits(max_failures: u32, window: Duration, capacity: usize) -> Self {
        Self {
            max_failures,
            window,
            capacity,
            failures: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ThrottleKey, Failures>> {
        // The map holds plain counters, so a panic elsewhere cannot leave it inconsistent.
        self.failures.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Fails while the account has used up its failures for the current window.
    pub fn check(&self, key: &ThrottleKey) -> Result<(), IdentityError> {
        let map = self.lock();
        match map.get(key) {
            Some(f) if f.count >= self.max_failures && f.window_start.elapsed() < self.window => {
                Err(IdentityError::TooManyAttempts {
                    retry_after_secs: (self.window - f.window_start.elapsed()).as_secs().max(1),
                })
            }
            _ => Ok(()),
        }
    }

    pub fn record_failure(&self, key: &ThrottleKey) {
        let mut map = self.lock();
        let now = Instant::now();
        if let Some(entry) = map.get_mut(key) {
            if entry.window_start.elapsed() < self.window {
                entry.count = entry.count.saturating_add(1);
            } else {
                *entry = Failures {
                    count: 1,
                    window_start: now,
                };
            }
            return;
        }
        if map.len() >= self.capacity {
            self.make_room(&mut map);
        }
        map.insert(
            *key,
            Failures {
                count: 1,
                window_start: now,
            },
        );
    }

    pub fn reset(&self, key: &ThrottleKey) {
        self.lock().remove(key);
    }

    /// Drops expired entries and, if the map is still full, the oldest tenth, so spraying
    /// random emails cannot grow memory and a full map is not rescanned on every insert.
    fn make_room(&self, map: &mut HashMap<ThrottleKey, Failures>) {
        map.retain(|_, f| f.window_start.elapsed() < self.window);
        if map.len() < self.capacity {
            return;
        }
        let mut by_age: Vec<(Instant, ThrottleKey)> =
            map.iter().map(|(k, f)| (f.window_start, *k)).collect();
        by_age.sort_unstable_by_key(|(started, _)| *started);
        for (_, key) in by_age.into_iter().take((self.capacity / 10).max(1)) {
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
/// password is a generated temporary one). For the server CLI and the admin API, which has
/// already authorized.
pub async fn create_user(
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

// ---------------------------------------------------------------------------------------------
// Login and sessions
// ---------------------------------------------------------------------------------------------

/// A login in progress: the account lookup is done, the password is not checked yet.
pub struct LoginAttempt {
    key: ThrottleKey,
    record: Option<LoginRecord>,
}

/// First half of a login: refuses throttled accounts before any hashing, then loads the user.
pub async fn lookup_login(
    conn: &mut PgConnection,
    throttle: &LoginThrottle,
    email: &str,
) -> Result<LoginAttempt, IdentityError> {
    let key = ThrottleKey::new(email);
    throttle.check(&key)?;
    let record = match super::model::Email::parse(email) {
        Ok(email) => repo::find_login(conn, email.as_str()).await?,
        Err(_) => None,
    };
    Ok(LoginAttempt { key, record })
}

/// Second half of a login, with no database access: verifies the password (a dummy hash when
/// there is no account) and does the throttle bookkeeping. Every failure is the same error.
pub async fn complete_login(
    throttle: &LoginThrottle,
    attempt: LoginAttempt,
    password: &str,
) -> Result<User, IdentityError> {
    let LoginAttempt { key, record } = attempt;
    let hash = record.as_ref().map(|r| r.password_hash.clone());
    let verified = verify_password(password.to_owned(), hash).await?;
    match record {
        Some(record) if verified && record.user.status == UserStatus::Active => {
            throttle.reset(&key);
            Ok(record.user)
        }
        _ => {
            throttle.record_failure(&key);
            Err(IdentityError::InvalidCredentials)
        }
    }
}

pub async fn create_session(
    conn: &mut PgConnection,
    user: &User,
    kind: SessionKind,
) -> Result<NewSession, IdentityError> {
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
// Own account
// ---------------------------------------------------------------------------------------------

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
/// (counting failures against the account's login throttle, since a stolen session could
/// otherwise guess it), applies the policy and hashes the new one.
pub async fn prepare_password_change(
    throttle: &LoginThrottle,
    context: PasswordChangeContext,
    current: &str,
    new: String,
    min_len: usize,
    institution: &str,
) -> Result<PreparedPasswordChange, IdentityError> {
    let PasswordChangeContext { user, old_hash } = context;
    let key = ThrottleKey::new(&user.email);
    throttle.check(&key)?;
    if !verify_password(current.to_owned(), Some(old_hash.clone())).await? {
        throttle.record_failure(&key);
        return Err(IdentityError::WrongCurrentPassword);
    }
    throttle.reset(&key);
    if new == current {
        return Err(IdentityError::PasswordUnchanged);
    }
    let policy = PasswordPolicy::new(min_len, &user.email, &user.display_name, institution);
    let password = Password::parse(new, &policy)?;
    let new_hash = hash_password(password.expose()).await?;
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
mod tests {
    use super::*;

    fn key(email: &str) -> ThrottleKey {
        ThrottleKey::new(email)
    }

    fn denied(throttle: &LoginThrottle, email: &str) -> bool {
        matches!(
            throttle.check(&key(email)),
            Err(IdentityError::TooManyAttempts { .. })
        )
    }

    #[test]
    fn throttle_blocks_after_the_tenth_failure_only() {
        let throttle = LoginThrottle::default();
        for _ in 0..9 {
            throttle.record_failure(&key("a@x.edu"));
        }
        assert!(!denied(&throttle, "a@x.edu"));
        throttle.record_failure(&key("a@x.edu"));
        assert!(denied(&throttle, "a@x.edu"));
        assert!(
            !denied(&throttle, "b@x.edu"),
            "other accounts are unaffected"
        );
    }

    #[test]
    fn throttle_key_ignores_case_and_surrounding_spaces() {
        let throttle = LoginThrottle::with_limits(1, Duration::from_secs(60), 10);
        throttle.record_failure(&key(" A@X.edu "));
        assert!(denied(&throttle, "a@x.edu"));
    }

    #[test]
    fn success_resets_the_counter() {
        let throttle = LoginThrottle::with_limits(2, Duration::from_secs(60), 10);
        throttle.record_failure(&key("a@x.edu"));
        throttle.reset(&key("a@x.edu"));
        throttle.record_failure(&key("a@x.edu"));
        assert!(!denied(&throttle, "a@x.edu"));
    }

    #[test]
    fn throttle_window_expires() {
        let throttle = LoginThrottle::with_limits(1, Duration::from_millis(40), 10);
        throttle.record_failure(&key("a@x.edu"));
        assert!(denied(&throttle, "a@x.edu"));
        std::thread::sleep(Duration::from_millis(60));
        assert!(!denied(&throttle, "a@x.edu"));
        throttle.record_failure(&key("a@x.edu"));
        assert!(
            denied(&throttle, "a@x.edu"),
            "a new window starts counting again"
        );
    }

    #[test]
    fn retry_after_is_reported() {
        let throttle = LoginThrottle::with_limits(1, Duration::from_secs(60), 10);
        throttle.record_failure(&key("a@x.edu"));
        match throttle.check(&key("a@x.edu")) {
            Err(IdentityError::TooManyAttempts { retry_after_secs }) => {
                assert!((1..=60).contains(&retry_after_secs));
            }
            other => panic!("expected a throttle error, got {other:?}"),
        }
    }

    #[test]
    fn throttle_memory_is_bounded() {
        let throttle = LoginThrottle::with_limits(10, Duration::from_secs(60), 100);
        for i in 0..1_000 {
            throttle.record_failure(&key(&format!("user{i}@x.edu")));
            assert!(throttle.len() <= 100);
        }
    }

    #[test]
    fn full_throttle_drops_expired_entries_before_live_ones() {
        let throttle = LoginThrottle::with_limits(1, Duration::from_millis(40), 3);
        throttle.record_failure(&key("old1@x.edu"));
        throttle.record_failure(&key("old2@x.edu"));
        std::thread::sleep(Duration::from_millis(60));
        throttle.record_failure(&key("live@x.edu"));
        throttle.record_failure(&key("new@x.edu"));
        assert!(denied(&throttle, "live@x.edu"));
        assert!(denied(&throttle, "new@x.edu"));
        assert_eq!(throttle.len(), 2);
    }

    #[tokio::test]
    async fn hashes_verify_and_use_owasp_parameters() {
        let hash = hash_password("correct horse battery staple").await.unwrap();
        assert!(hash.as_str().starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert!(
            verify_password(
                "correct horse battery staple".into(),
                Some(hash.as_str().into())
            )
            .await
            .unwrap()
        );
        assert!(
            !verify_password("wrong".into(), Some(hash.as_str().into()))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn verifying_without_a_hash_is_false_but_still_hashes() {
        let started = Instant::now();
        assert!(!verify_password("anything".into(), None).await.unwrap());
        assert!(
            started.elapsed() > Duration::from_millis(1),
            "a dummy hash must have been computed"
        );
    }

    #[tokio::test]
    async fn malformed_stored_hash_is_a_mismatch() {
        assert!(
            !verify_password("pw".into(), Some("not a phc string".into()))
                .await
                .unwrap()
        );
    }
}
