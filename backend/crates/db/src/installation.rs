//! The installation record (ADR-0031): the institution mode, written by the first command that
//! uses the database and compared by every later one.

use sqlx::PgConnection;
use vumlo_config::InstitutionMode;

#[derive(Debug, thiserror::Error)]
pub enum ModeError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error("the database records the institution mode \"{recorded}\"")]
    Unknown { recorded: String },
    #[error(
        "institution.mode is \"{}\" in the configuration, but this installation was set up as \"{}\"; the mode cannot be changed (ADR-0031)",
        configured.as_str(),
        recorded.as_str()
    )]
    Changed {
        recorded: InstitutionMode,
        configured: InstitutionMode,
    },
}

/// Records the mode on the first start of the installation; on every later start checks that the
/// configuration still names it. Runs on the caller's transaction. Under READ COMMITTED a
/// concurrent first insert waits for the other transaction on the key, then does nothing, so
/// exactly one mode is ever stored.
pub async fn record_mode(
    conn: &mut PgConnection,
    configured: InstitutionMode,
) -> Result<(), ModeError> {
    sqlx::query!(
        "INSERT INTO org.installation (mode) VALUES ($1) ON CONFLICT (singleton) DO NOTHING",
        configured.as_str()
    )
    .execute(&mut *conn)
    .await?;
    let row = sqlx::query!(r#"SELECT mode AS "mode!" FROM org.installation"#)
        .fetch_one(&mut *conn)
        .await?;
    let recorded: InstitutionMode = row
        .mode
        .parse()
        .map_err(|_| ModeError::Unknown { recorded: row.mode })?;
    if recorded == configured {
        Ok(())
    } else {
        Err(ModeError::Changed {
            recorded,
            configured,
        })
    }
}
