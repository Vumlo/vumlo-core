//! SQL only. Every function takes the connection it runs on and never begins a transaction.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{
    NewUser, Role, RoleCounts, SessionInfo, SessionKind, TokenPurpose, User, UserFilter, UserStatus,
};

pub struct LoginRecord {
    pub user: User,
    pub password_hash: String,
    pub email_verified: bool,
}

/// What registration, the reset request and the mail handler need to know about an account.
pub struct AccountRow {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub surname: String,
    pub role: Role,
    pub status: UserStatus,
    pub email_verified: bool,
}

pub struct SessionLookup {
    pub session_id: Uuid,
    pub kind: SessionKind,
    /// `last_seen_at` is more than an hour old, so the session should be refreshed.
    pub stale: bool,
    pub user_id: Uuid,
    pub role: Role,
    pub must_change_password: bool,
}

/// The user with a password identity for `email` (already normalized), if any.
pub async fn find_login(
    conn: &mut PgConnection,
    email: &str,
) -> Result<Option<LoginRecord>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT u.id, u.email, u.name, u.surname,
                  u.role AS "role: Role", u.status AS "status: UserStatus",
                  u.must_change_password, u.created_at,
                  i.password_hash AS "password_hash!",
                  u.email_verified_at IS NOT NULL AS "email_verified!"
           FROM identity.users u
           JOIN identity.identities i ON i.user_id = u.id AND i.provider = 'password'
           WHERE u.email = $1"#,
        email
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|r| LoginRecord {
        user: User {
            id: r.id,
            email: r.email,
            name: r.name,
            surname: r.surname,
            role: r.role,
            status: r.status,
            must_change_password: r.must_change_password,
            created_at: r.created_at,
        },
        password_hash: r.password_hash,
        email_verified: r.email_verified,
    }))
}

/// Locks the password identity and the user `FOR SHARE` until the transaction ends and says
/// whether the account is still active and the password hash is still `password_hash`. A change of
/// either waits for this transaction (ADR-0038).
///
/// The identity is locked first, then the user: the order a password reset writes them in. One statement
/// over both (`FOR SHARE OF u, i`) took them in the planner's order, and a login that held the user while it
/// waited for the identity deadlocked with a reset that held the identity and wanted the user.
pub async fn lock_verified_login(
    conn: &mut PgConnection,
    user_id: Uuid,
    password_hash: &str,
) -> Result<bool, sqlx::Error> {
    let identity = sqlx::query_scalar!(
        r#"SELECT 1 AS "locked!"
           FROM identity.identities
           WHERE user_id = $1 AND provider = 'password' AND password_hash = $2
           FOR SHARE"#,
        user_id,
        password_hash
    )
    .fetch_optional(&mut *conn)
    .await?;
    if identity.is_none() {
        return Ok(false);
    }
    let user = sqlx::query_scalar!(
        r#"SELECT 1 AS "locked!" FROM identity.users WHERE id = $1 AND status = 'active' FOR SHARE"#,
        user_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(user.is_some())
}

pub async fn insert_user(
    conn: &mut PgConnection,
    id: Uuid,
    new: &NewUser,
    must_change_password: bool,
) -> Result<User, sqlx::Error> {
    sqlx::query_as!(
        User,
        r#"INSERT INTO identity.users
               (id, email, name, surname, role, must_change_password, email_verified_at)
           VALUES ($1, $2, $3, $4, $5, $6, now())
           RETURNING id, email, name, surname,
                     role AS "role: Role", status AS "status: UserStatus",
                     must_change_password, created_at"#,
        id,
        new.email.as_str(),
        new.name.as_str(),
        new.surname.as_str(),
        new.role.as_str(),
        must_change_password
    )
    .fetch_one(conn)
    .await
}

pub async fn insert_password_identity(
    conn: &mut PgConnection,
    user_id: Uuid,
    password_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO identity.identities (id, user_id, provider, subject, password_hash)
         VALUES ($1, $2, 'password', $3, $4)",
        Uuid::now_v7(),
        user_id,
        user_id.to_string(),
        password_hash
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Stores a session and returns its expiry, computed with the database clock.
pub async fn insert_session(
    conn: &mut PgConnection,
    user_id: Uuid,
    token_hash: &[u8],
    kind: SessionKind,
    lifetime_secs: i64,
) -> Result<DateTime<Utc>, sqlx::Error> {
    let expires_at = sqlx::query_scalar!(
        "INSERT INTO identity.sessions (id, user_id, token_hash, kind, expires_at)
         VALUES ($1, $2, $3, $4, now() + $5::bigint * interval '1 second')
         RETURNING expires_at",
        Uuid::now_v7(),
        user_id,
        token_hash,
        kind.as_str(),
        lifetime_secs
    )
    .fetch_one(&mut *conn)
    .await?;
    // Every new session is a sign-in (ADR-0053); a refreshed one is not.
    sqlx::query!(
        "INSERT INTO identity.last_logins (user_id, at) VALUES ($1, now())
         ON CONFLICT (user_id) DO UPDATE SET at = EXCLUDED.at",
        user_id
    )
    .execute(conn)
    .await?;
    Ok(expires_at)
}

/// People by institution role (all statuses), and how many are disabled (ADR-0053).
pub async fn role_counts(conn: &mut PgConnection) -> Result<RoleCounts, sqlx::Error> {
    sqlx::query_as!(
        RoleCounts,
        r#"SELECT count(*) AS "all!",
                  count(*) FILTER (WHERE role = 'student') AS "student!",
                  count(*) FILTER (WHERE role = 'teacher') AS "teacher!",
                  count(*) FILTER (WHERE role = 'admin') AS "admin!",
                  count(*) FILTER (WHERE status = 'disabled') AS "disabled!"
           FROM identity.users"#
    )
    .fetch_one(conn)
    .await
}

/// When the person last signed in; `None` if never since the time was recorded.
pub async fn last_login_at(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT at FROM identity.last_logins WHERE user_id = $1",
        user_id
    )
    .fetch_optional(conn)
    .await
}

/// The live session for a token hash, if its user is active and it has not expired.
pub async fn find_session(
    conn: &mut PgConnection,
    token_hash: &[u8],
) -> Result<Option<SessionLookup>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT s.id AS session_id, s.kind AS "kind: SessionKind",
                  s.last_seen_at < now() - interval '1 hour' AS "stale!",
                  u.id AS user_id, u.role AS "role: Role", u.must_change_password
           FROM identity.sessions s
           JOIN identity.users u ON u.id = s.user_id
           WHERE s.token_hash = $1 AND s.expires_at > now() AND u.status = 'active'"#,
        token_hash
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|r| SessionLookup {
        session_id: r.session_id,
        kind: r.kind,
        stale: r.stale,
        user_id: r.user_id,
        role: r.role,
        must_change_password: r.must_change_password,
    }))
}

/// Marks the session as seen and, for sliding kinds, pushes the expiry forward. Does nothing
/// when it was refreshed within the last hour, so concurrent requests write at most once.
pub async fn touch_session(
    conn: &mut PgConnection,
    session_id: Uuid,
    slide_secs: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE identity.sessions
         SET last_seen_at = now(),
             expires_at = CASE WHEN $2::bigint IS NULL THEN expires_at
                               ELSE now() + $2::bigint * interval '1 second' END
         WHERE id = $1 AND last_seen_at < now() - interval '1 hour'",
        session_id,
        slide_secs
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn delete_session(
    conn: &mut PgConnection,
    session_id: Uuid,
    user_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM identity.sessions WHERE id = $1 AND user_id = $2",
        session_id,
        user_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

pub async fn find_user(conn: &mut PgConnection, id: Uuid) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as!(
        User,
        r#"SELECT id, email, name, surname,
                  role AS "role: Role", status AS "status: UserStatus",
                  must_change_password, created_at
           FROM identity.users WHERE id = $1"#,
        id
    )
    .fetch_optional(conn)
    .await
}

/// The user, with their row locked `FOR SHARE` until the transaction ends. A role or status change
/// of that user waits for this transaction, and a transaction that began after the change sees it
/// (ADR-0037): the caller reads the role in the same statement that holds the lock.
pub async fn find_user_shared(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as!(
        User,
        r#"SELECT id, email, name, surname,
                  role AS "role: Role", status AS "status: UserStatus",
                  must_change_password, created_at
           FROM identity.users WHERE id = $1
           FOR SHARE"#,
        id
    )
    .fetch_optional(conn)
    .await
}

pub async fn find_users(conn: &mut PgConnection, ids: &[Uuid]) -> Result<Vec<User>, sqlx::Error> {
    sqlx::query_as!(
        User,
        r#"SELECT id, email, name, surname,
                  role AS "role: Role", status AS "status: UserStatus",
                  must_change_password, created_at
           FROM identity.users WHERE id = ANY($1)"#,
        ids
    )
    .fetch_all(conn)
    .await
}

pub async fn find_password_hash(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT password_hash FROM identity.identities
         WHERE user_id = $1 AND provider = 'password'",
        user_id
    )
    .fetch_optional(conn)
    .await
    .map(Option::flatten)
}

/// Replaces the password hash only if it is still `old`, so two concurrent changes (or a change
/// racing an admin reset) cannot overwrite each other silently. Returns the rows changed.
pub async fn swap_password_hash(
    conn: &mut PgConnection,
    user_id: Uuid,
    old: &str,
    new: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "UPDATE identity.identities SET password_hash = $3
         WHERE user_id = $1 AND provider = 'password' AND password_hash = $2",
        user_id,
        old,
        new
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

pub async fn set_must_change_password(
    conn: &mut PgConnection,
    user_id: Uuid,
    value: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE identity.users SET must_change_password = $2, updated_at = now() WHERE id = $1",
        user_id,
        value
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn delete_other_sessions(
    conn: &mut PgConnection,
    user_id: Uuid,
    keep_session_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM identity.sessions WHERE user_id = $1 AND id <> $2",
        user_id,
        keep_session_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// The user's unexpired sessions after `after`, in id order, at most `limit`.
pub async fn list_sessions(
    conn: &mut PgConnection,
    user_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<SessionInfo>, sqlx::Error> {
    sqlx::query_as!(
        SessionInfo,
        r#"SELECT id, kind AS "kind: SessionKind", name, created_at, last_seen_at, expires_at
           FROM identity.sessions
           WHERE user_id = $1 AND expires_at > now() AND ($2::uuid IS NULL OR id > $2)
           ORDER BY id
           LIMIT $3"#,
        user_id,
        after,
        limit
    )
    .fetch_all(conn)
    .await
}

pub async fn list_users(
    conn: &mut PgConnection,
    filter: &UserFilter,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<User>, sqlx::Error> {
    // `strpos` instead of LIKE: the search text never needs escaping. Emails are stored lowercase.
    sqlx::query_as!(
        User,
        r#"SELECT id, email, name, surname,
                  role AS "role: Role", status AS "status: UserStatus",
                  must_change_password, created_at
           FROM identity.users
           WHERE ($1::text IS NULL OR role = $1)
             AND ($2::text IS NULL OR status = $2)
             AND ($3::text IS NULL OR strpos(email, $3) > 0)
             AND ($4::uuid IS NULL OR id > $4)
           ORDER BY id
           LIMIT $5"#,
        filter.role.map(Role::as_str),
        filter.status.map(UserStatus::as_str),
        filter.email_contains.as_deref(),
        after,
        limit
    )
    .fetch_all(conn)
    .await
}

/// Locks every active admin until the end of the transaction and returns their ids. Concurrent
/// demotions queue here, so the last active admin cannot be removed by two racing requests.
pub async fn lock_active_admins(conn: &mut PgConnection) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT id FROM identity.users WHERE role = 'admin' AND status = 'active' ORDER BY id FOR UPDATE"
    )
    .fetch_all(conn)
    .await
}

pub async fn update_user(
    conn: &mut PgConnection,
    id: Uuid,
    name: Option<&str>,
    surname: Option<&str>,
    role: Option<Role>,
    status: Option<UserStatus>,
) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as!(
        User,
        r#"UPDATE identity.users
           SET name = COALESCE($2, name),
               surname = COALESCE($3, surname),
               role = COALESCE($4, role),
               status = COALESCE($5, status),
               updated_at = now()
           WHERE id = $1
           RETURNING id, email, name, surname,
                     role AS "role: Role", status AS "status: UserStatus",
                     must_change_password, created_at"#,
        id,
        name,
        surname,
        role.map(Role::as_str),
        status.map(UserStatus::as_str)
    )
    .fetch_optional(conn)
    .await
}

pub async fn delete_sessions_of(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM identity.sessions WHERE user_id = $1", user_id)
        .execute(conn)
        .await?;
    Ok(result.rows_affected())
}

/// Sets the user's password, creating the password identity if they only had another provider.
pub async fn upsert_password_hash(
    conn: &mut PgConnection,
    user_id: Uuid,
    password_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO identity.identities (id, user_id, provider, subject, password_hash)
         VALUES ($1, $2, 'password', $3, $4)
         ON CONFLICT (provider, subject) DO UPDATE SET password_hash = EXCLUDED.password_hash",
        Uuid::now_v7(),
        user_id,
        user_id.to_string(),
        password_hash
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Creates the account of a self-registration: a `student` whose address is not verified. `None`
/// when the address already has an account, so concurrent registrations cannot fail each other.
pub async fn insert_registered_user(
    conn: &mut PgConnection,
    id: Uuid,
    new: &NewUser,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "INSERT INTO identity.users (id, email, name, surname, role, must_change_password)
         VALUES ($1, $2, $3, $4, 'student', false)
         ON CONFLICT (email) DO NOTHING
         RETURNING id",
        id,
        new.email.as_str(),
        new.name.as_str(),
        new.surname.as_str()
    )
    .fetch_optional(conn)
    .await
}

/// The account with this address (already normalized), locked until the transaction ends.
pub async fn lock_account_by_email(
    conn: &mut PgConnection,
    email: &str,
) -> Result<Option<AccountRow>, sqlx::Error> {
    sqlx::query_as!(
        AccountRow,
        r#"SELECT id, email, name, surname, role AS "role: Role", status AS "status: UserStatus",
                  email_verified_at IS NOT NULL AS "email_verified!"
           FROM identity.users WHERE email = $1 FOR UPDATE"#,
        email
    )
    .fetch_optional(conn)
    .await
}

pub async fn find_account(
    conn: &mut PgConnection,
    id: Uuid,
) -> Result<Option<AccountRow>, sqlx::Error> {
    sqlx::query_as!(
        AccountRow,
        r#"SELECT id, email, name, surname, role AS "role: Role", status AS "status: UserStatus",
                  email_verified_at IS NOT NULL AS "email_verified!"
           FROM identity.users WHERE id = $1"#,
        id
    )
    .fetch_optional(conn)
    .await
}

pub async fn update_names(
    conn: &mut PgConnection,
    id: Uuid,
    name: &str,
    surname: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE identity.users SET name = $2, surname = $3, updated_at = now() WHERE id = $1",
        id,
        name,
        surname
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn mark_email_verified(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE identity.users
         SET email_verified_at = COALESCE(email_verified_at, now()), updated_at = now()
         WHERE id = $1",
        id
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Voids the unused tokens of a user and purpose; returns how many there were.
pub async fn void_email_tokens(
    conn: &mut PgConnection,
    user_id: Uuid,
    purpose: TokenPurpose,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "UPDATE identity.email_tokens SET used_at = now()
         WHERE user_id = $1 AND purpose = $2 AND used_at IS NULL",
        user_id,
        purpose.as_str()
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

pub async fn insert_email_token(
    conn: &mut PgConnection,
    user_id: Uuid,
    purpose: TokenPurpose,
    token_hash: &[u8],
    lifetime_secs: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO identity.email_tokens (id, user_id, purpose, token_hash, expires_at)
         VALUES ($1, $2, $3, $4, now() + make_interval(secs => $5))",
        Uuid::now_v7(),
        user_id,
        purpose.as_str(),
        token_hash,
        lifetime_secs as f64
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Uses the token up: one conditional update, so of two requests with the same token exactly one
/// gets the user id. `None`: unknown, used or expired.
pub async fn consume_email_token(
    conn: &mut PgConnection,
    token_hash: &[u8],
    purpose: TokenPurpose,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "UPDATE identity.email_tokens SET used_at = now()
         WHERE token_hash = $1 AND purpose = $2 AND used_at IS NULL AND expires_at > now()
         RETURNING user_id",
        token_hash,
        purpose.as_str()
    )
    .fetch_optional(conn)
    .await
}

/// The account a usable token belongs to, without using the token up.
pub async fn find_token_account(
    conn: &mut PgConnection,
    token_hash: &[u8],
    purpose: TokenPurpose,
) -> Result<Option<AccountRow>, sqlx::Error> {
    sqlx::query_as!(
        AccountRow,
        r#"SELECT u.id, u.email, u.name, u.surname, u.role AS "role: Role",
                  u.status AS "status: UserStatus",
                  u.email_verified_at IS NOT NULL AS "email_verified!"
           FROM identity.email_tokens t
           JOIN identity.users u ON u.id = t.user_id
           WHERE t.token_hash = $1 AND t.purpose = $2 AND t.used_at IS NULL AND t.expires_at > now()"#,
        token_hash,
        purpose.as_str()
    )
    .fetch_optional(conn)
    .await
}

/// Stores one started Google sign-in and forgets the expired ones (ADR-0047). It is tied to a browser or
/// to an app (ADR-0062), never both.
pub async fn insert_oauth_state(
    conn: &mut PgConnection,
    state_hash: &[u8],
    browser_hash: Option<&[u8]>,
    app_challenge: Option<&[u8]>,
    nonce: &str,
    pkce_verifier: &str,
    lifetime_secs: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM identity.oauth_states WHERE expires_at < now()")
        .execute(&mut *conn)
        .await?;
    sqlx::query!(
        "INSERT INTO identity.oauth_states
             (state_hash, browser_hash, app_challenge, nonce, pkce_verifier, expires_at)
         VALUES ($1, $2, $3, $4, $5, now() + make_interval(secs => $6))",
        state_hash,
        browser_hash,
        app_challenge,
        nonce,
        pkce_verifier,
        lifetime_secs as f64
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// What a started sign-in remembered.
pub struct TakenState {
    pub nonce: String,
    pub pkce_verifier: String,
    /// Set when an app started it.
    pub app_challenge: Option<Vec<u8>>,
}

/// Reads and deletes the state in one statement, so it can be used once. A browser's attempt matches
/// only for the browser that started it; an app's attempt needs no cookie (the app holds the secret).
/// Both only before they expire.
pub async fn take_oauth_state(
    conn: &mut PgConnection,
    state_hash: &[u8],
    browser_hash: Option<&[u8]>,
) -> Result<Option<TakenState>, sqlx::Error> {
    let row = sqlx::query!(
        "DELETE FROM identity.oauth_states
         WHERE state_hash = $1 AND expires_at > now()
           AND (app_challenge IS NOT NULL OR browser_hash = $2)
         RETURNING nonce, pkce_verifier, app_challenge",
        state_hash,
        browser_hash
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|row| TakenState {
        nonce: row.nonce,
        pkce_verifier: row.pkce_verifier,
        app_challenge: row.app_challenge,
    }))
}

/// Keeps the handoff of a finished app sign-in and forgets the expired ones.
pub async fn insert_app_handoff(
    conn: &mut PgConnection,
    code_hash: &[u8],
    user_id: Uuid,
    app_challenge: &[u8],
    lifetime_secs: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM identity.app_handoffs WHERE expires_at < now()")
        .execute(&mut *conn)
        .await?;
    sqlx::query!(
        "INSERT INTO identity.app_handoffs (code_hash, user_id, app_challenge, expires_at)
         VALUES ($1, $2, $3, now() + make_interval(secs => $4))",
        code_hash,
        user_id,
        app_challenge,
        lifetime_secs as f64
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Reads and deletes a handoff, whoever asks and whatever they bring with it: a code that is guessed at
/// is spent, and the one that matches is used once.
pub async fn take_app_handoff(
    conn: &mut PgConnection,
    code_hash: &[u8],
) -> Result<Option<(Uuid, Vec<u8>)>, sqlx::Error> {
    let row = sqlx::query!(
        "DELETE FROM identity.app_handoffs WHERE code_hash = $1 AND expires_at > now()
         RETURNING user_id, app_challenge",
        code_hash
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|row| (row.user_id, row.app_challenge)))
}

/// The user a Google subject is linked to, locked with the row of the identity.
pub async fn find_google_user(
    conn: &mut PgConnection,
    subject: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT user_id FROM identity.identities WHERE provider = 'google' AND subject = $1",
        subject
    )
    .fetch_optional(conn)
    .await
}

pub async fn insert_google_identity(
    conn: &mut PgConnection,
    user_id: Uuid,
    subject: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO identity.identities (id, user_id, provider, subject)
         VALUES ($1, $2, 'google', $3)",
        Uuid::now_v7(),
        user_id,
        subject
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn delete_password_identity(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "DELETE FROM identity.identities WHERE user_id = $1 AND provider = 'password'",
        user_id
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Locks the user `FOR SHARE` and says whether the account is active (ADR-0038).
pub async fn lock_active_user(conn: &mut PgConnection, user_id: Uuid) -> Result<bool, sqlx::Error> {
    let row = sqlx::query_scalar!(
        r#"SELECT 1 AS "locked!" FROM identity.users
           WHERE id = $1 AND status = 'active' FOR SHARE"#,
        user_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.is_some())
}

pub async fn find_account_by_email(
    conn: &mut PgConnection,
    email: &str,
) -> Result<Option<AccountRow>, sqlx::Error> {
    sqlx::query_as!(
        AccountRow,
        r#"SELECT id, email, name, surname, role AS "role: Role", status AS "status: UserStatus",
                  email_verified_at IS NOT NULL AS "email_verified!"
           FROM identity.users WHERE email = $1"#,
        email
    )
    .fetch_optional(conn)
    .await
}
