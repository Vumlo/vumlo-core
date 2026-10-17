//! SQL only. Every function takes the connection it runs on and never begins a transaction.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{NewUser, Role, SessionInfo, SessionKind, User, UserFilter, UserStatus};

pub struct LoginRecord {
    pub user: User,
    pub password_hash: String,
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
                  i.password_hash AS "password_hash!"
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
    }))
}

pub async fn insert_user(
    conn: &mut PgConnection,
    id: Uuid,
    new: &NewUser,
    must_change_password: bool,
) -> Result<User, sqlx::Error> {
    sqlx::query_as!(
        User,
        r#"INSERT INTO identity.users (id, email, name, surname, role, must_change_password)
           VALUES ($1, $2, $3, $4, $5, $6)
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
    sqlx::query_scalar!(
        "INSERT INTO identity.sessions (id, user_id, token_hash, kind, expires_at)
         VALUES ($1, $2, $3, $4, now() + $5::bigint * interval '1 second')
         RETURNING expires_at",
        Uuid::now_v7(),
        user_id,
        token_hash,
        kind.as_str(),
        lifetime_secs
    )
    .fetch_one(conn)
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
