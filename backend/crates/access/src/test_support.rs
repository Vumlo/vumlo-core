use common::Role;
use uuid::Uuid;

use super::Actor;

impl Actor {
    /// An actor for a test, built without a session. Not compiled into the server: only the
    /// extractors build an `Actor` there.
    pub fn for_tests(user_id: Uuid, role: Role, session_id: Uuid) -> Self {
        Self::new(user_id, role, session_id)
    }
}
