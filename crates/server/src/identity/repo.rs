//! SQL only. Every function takes the connection it runs on and never begins a transaction.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{NewUser, Role, SessionKind, User, UserStatus};

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
        r#"SELECT u.id, u.email, u.display_name,
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
            display_name: r.display_name,
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
        r#"INSERT INTO identity.users (id, email, display_name, role, must_change_password)
           VALUES ($1, $2, $3, $4, $5)
           RETURNING id, email, display_name,
                     role AS "role: Role", status AS "status: UserStatus",
                     must_change_password, created_at"#,
        id,
        new.email.as_str(),
        new.display_name.as_str(),
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
