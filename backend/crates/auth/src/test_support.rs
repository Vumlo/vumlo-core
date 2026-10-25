use sqlx::PgConnection;

use super::service::create_user;
use super::{AuthError, HashedPassword, NewUser, User};

/// Seeds a user for the unit tests of any crate above, with no audit row. Not compiled into the
/// server: production code creates users only through the audited functions (ADR-0034).
pub async fn create_user_unaudited_for_tests(
    conn: &mut PgConnection,
    new: NewUser,
    password: HashedPassword,
) -> Result<User, AuthError> {
    create_user(conn, new, password).await
}
