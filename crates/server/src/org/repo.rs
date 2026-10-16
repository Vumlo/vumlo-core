//! SQL only. Every function takes the connection it runs on and never begins a transaction.

use sqlx::PgConnection;

use crate::settings::InstitutionMode;

/// Writes the mode if the installation has none yet; leaves an existing record alone. Under READ
/// COMMITTED a concurrent first insert waits for the other transaction on the key, then does
/// nothing, so exactly one mode is ever stored.
pub async fn insert_mode_if_absent(
    conn: &mut PgConnection,
    mode: InstitutionMode,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO org.installation (mode) VALUES ($1) ON CONFLICT (singleton) DO NOTHING",
        mode.as_str()
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The recorded mode. A statement after the insert takes a fresh snapshot, so it sees the row a
/// concurrent starter committed.
pub async fn recorded_mode(conn: &mut PgConnection) -> Result<InstitutionMode, sqlx::Error> {
    let row = sqlx::query!(r#"SELECT mode AS "mode: InstitutionMode" FROM org.installation"#)
        .fetch_one(conn)
        .await?;
    Ok(row.mode)
}
