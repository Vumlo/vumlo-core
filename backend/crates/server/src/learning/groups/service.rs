use sqlx::PgConnection;
use uuid::Uuid;

use super::model::GroupOverview;
use crate::access::{self, Action};
use crate::actor::Actor;
use crate::learning::LearningError;
use crate::learning::teaching;
use crate::org;

/// A group with its people and the courses that use it (ADR-0053). Admins only.
pub async fn group_overview(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<GroupOverview, LearningError> {
    access::authorize(actor, Action::ViewGroups)?;
    let members = org::group_member_lines(conn, actor, id).await?;
    let linked_courses = teaching::linked_courses_of_group(conn, id).await?;
    Ok(GroupOverview {
        group_id: id,
        members,
        linked_courses,
    })
}
