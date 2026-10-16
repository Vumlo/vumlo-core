use sqlx::PgConnection;

use super::repo;
use crate::settings::InstitutionMode;

#[derive(Debug, thiserror::Error)]
pub enum OrgError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(
        "institution.mode is \"{}\" in the configuration, but this installation was set up as \"{}\"; the mode cannot be changed (ADR-0031)",
        configured.as_str(),
        recorded.as_str()
    )]
    ModeChanged {
        recorded: InstitutionMode,
        configured: InstitutionMode,
    },
}

/// Records the mode on the first start of the installation; on every later start checks that the
/// configuration still names it (ADR-0031). Acts for no user, so it takes no `Actor`. Runs on the
/// caller's transaction.
pub async fn record_mode(
    conn: &mut PgConnection,
    configured: InstitutionMode,
) -> Result<(), OrgError> {
    repo::insert_mode_if_absent(conn, configured).await?;
    let recorded = repo::recorded_mode(conn).await?;
    if recorded == configured {
        Ok(())
    } else {
        Err(OrgError::ModeChanged {
            recorded,
            configured,
        })
    }
}
