use access::Actor;
use sqlx::PgConnection;
use uuid::Uuid;

use super::OrgError;
use super::service::{add_group_members, create_group};

/// A group with these students in it, made the way an admin makes one. For the tests of other
/// crates that need a group; they must not touch `org`'s tables themselves.
pub async fn seed_group(
    conn: &mut PgConnection,
    admin: &Actor,
    name: &str,
    students: &[Uuid],
) -> Result<Uuid, OrgError> {
    let group = create_group(conn, admin, name).await?;
    add_group_members(conn, admin, group.id, students).await?;
    Ok(group.id)
}
