//! Business rules. Every function that acts for a user authorizes first, takes the connection
//! it runs on and never begins or commits a transaction. Password hashing is exposed separately
//! so callers can do it before they open one.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::{Algorithm, Argon2, Params, Version};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{
    HashedPassword, ModelError, NewSession, NewUser, Password, PasswordPolicy, PersonName, Role,
    SessionInfo, SessionKind, SessionToken, TemporaryPassword, User, UserFilter, UserStatus,
    UserUpdate,
};
use super::repo::{self, LoginRecord};
use crate::actor::Actor;
use crate::events::{self, AuditEntry, EventsError};
use crate::pagination::{Page, PageParams};
use crate::policy::{self, Action, PolicyError};
use crate::teaching::{self, TeachingError};

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
    #[error("the last active admin cannot be demoted or disabled")]
    LastAdmin,
    /// A teacher or admin cannot become a student while they own a course (ADR-0037).
    #[error("the user owns courses")]
    OwnsCourses,
    #[error(transparent)]
    Teaching(Box<TeachingError>),
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
    let policy = PasswordPolicy::new(min_len, &user.email, &user.name, &user.surname, institution);
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
    use crate::test_support::{db_test, wait_until_blocked};

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

    async fn admin_actor(pool: &sqlx::PgPool, email: &str) -> Actor {
        use super::super::model::{Email, PersonName};
        let user = repo::insert_user(
            &mut pool.acquire().await.unwrap(),
            Uuid::now_v7(),
            &NewUser {
                email: Email::parse(email).unwrap(),
                name: PersonName::name("Admin").unwrap(),
                surname: PersonName::surname("Admin").unwrap(),
                role: Role::Admin,
            },
            false,
        )
        .await
        .unwrap();
        Actor::new(user.id, Role::Admin, Uuid::now_v7())
    }

    fn demote() -> UserUpdate {
        UserUpdate {
            role: Some(Role::Teacher),
            ..UserUpdate::default()
        }
    }

    #[tokio::test]
    async fn concurrent_demotions_cannot_remove_every_admin() {
        db_test(async |pool| {
            let a = admin_actor(&pool, "a@x.edu").await;
            let b = admin_actor(&pool, "b@x.edu").await;

            // `a` demotes `b` and holds its transaction open; `b` tries to demote `a` meanwhile.
            let mut first = pool.begin().await.unwrap();
            update_user(&mut first, &a, b.user_id(), demote())
                .await
                .unwrap();
            let second = tokio::spawn({
                let pool = pool.clone();
                let (b, a_id) = (b.clone(), a.user_id());
                async move {
                    let mut tx = pool.begin().await.unwrap();
                    update_user(&mut tx, &b, a_id, demote()).await
                }
            });
            wait_until_blocked(&pool, &second).await;
            first.commit().await.unwrap();

            let outcome = second.await.unwrap();
            assert!(
                matches!(outcome, Err(IdentityError::LastAdmin)),
                "{outcome:?}"
            );
            let admins = repo::lock_active_admins(&mut pool.acquire().await.unwrap())
                .await
                .unwrap();
            assert_eq!(admins, [a.user_id()]);
        })
        .await;
    }

    fn to_student() -> UserUpdate {
        UserUpdate {
            role: Some(Role::Student),
            ..UserUpdate::default()
        }
    }

    async fn teacher_actor(pool: &sqlx::PgPool, email: &str) -> Actor {
        use super::super::model::{Email, PersonName};
        let user = repo::insert_user(
            &mut pool.acquire().await.unwrap(),
            Uuid::now_v7(),
            &NewUser {
                email: Email::parse(email).unwrap(),
                name: PersonName::name("Teacher").unwrap(),
                surname: PersonName::surname("Teacher").unwrap(),
                role: Role::Teacher,
            },
            false,
        )
        .await
        .unwrap();
        Actor::new(user.id, Role::Teacher, Uuid::now_v7())
    }

    async fn course_of(pool: &sqlx::PgPool, owner: &Actor) -> Uuid {
        let mut tx = pool.begin().await.unwrap();
        let course = teaching::create_course(
            &mut tx,
            owner,
            teaching::CourseTitle::parse("Algebra").unwrap(),
            None,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        course.id
    }

    /// A member write holds the user's row `FOR SHARE` from the moment it reads the role; the
    /// demotion that comes meanwhile waits, and then removes the membership the write added.
    #[tokio::test]
    async fn a_demotion_waits_for_a_member_write_and_then_removes_the_membership() {
        db_test(async |pool| {
            let admin = admin_actor(&pool, "a@x.edu").await;
            let owner = teacher_actor(&pool, "owner@x.edu").await;
            let candidate = teacher_actor(&pool, "candidate@x.edu").await;
            let course = course_of(&pool, &owner).await;

            let mut first = pool.begin().await.unwrap();
            teaching::put_member(
                &mut first,
                &owner,
                course,
                candidate.user_id(),
                crate::policy::CourseRole::Teacher,
            )
            .await
            .unwrap();
            let second = tokio::spawn({
                let pool = pool.clone();
                let (admin, who) = (admin.clone(), candidate.user_id());
                async move {
                    let mut tx = pool.begin().await.unwrap();
                    update_user(&mut tx, &admin, who, to_student()).await?;
                    tx.commit().await.map_err(IdentityError::from)
                }
            });
            wait_until_blocked(&pool, &second).await;
            first.commit().await.unwrap();
            second.await.unwrap().unwrap();

            // The course is a draft: only a member who is staff can still see it.
            let access = teaching::course_access(
                &mut pool.acquire().await.unwrap(),
                &Actor::new(candidate.user_id(), Role::Student, Uuid::now_v7()),
                course,
            )
            .await;
            assert!(
                matches!(access, Err(TeachingError::CourseNotFound)),
                "no teacher membership survives the demotion: {access:?}"
            );
        })
        .await;
    }

    /// The other order: the demotion is in flight, and a member write that comes meanwhile waits
    /// for it and then reads the new role.
    #[tokio::test]
    async fn a_member_write_waits_for_a_demotion_and_then_refuses_the_teacher_role() {
        db_test(async |pool| {
            let admin = admin_actor(&pool, "a@x.edu").await;
            let owner = teacher_actor(&pool, "owner@x.edu").await;
            let candidate = teacher_actor(&pool, "candidate@x.edu").await;
            let course = course_of(&pool, &owner).await;

            let mut first = pool.begin().await.unwrap();
            update_user(&mut first, &admin, candidate.user_id(), to_student())
                .await
                .unwrap();
            let second = tokio::spawn({
                let pool = pool.clone();
                let (owner, who) = (owner.clone(), candidate.user_id());
                async move {
                    let mut tx = pool.begin().await.unwrap();
                    teaching::put_member(
                        &mut tx,
                        &owner,
                        course,
                        who,
                        crate::policy::CourseRole::Teacher,
                    )
                    .await
                }
            });
            wait_until_blocked(&pool, &second).await;
            first.commit().await.unwrap();

            let outcome = second.await.unwrap();
            assert!(
                matches!(outcome, Err(TeachingError::RoleNotAllowed)),
                "{outcome:?}"
            );
        })
        .await;
    }
}
