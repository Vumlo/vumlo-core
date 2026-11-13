use common::Role;
use uuid::Uuid;

use super::{Actor, Audience};

impl Actor {
    /// An actor for a test, built without a session, in the app. Not compiled into the server: only
    /// the extractors build an `Actor` there.
    pub fn for_tests(user_id: Uuid, role: Role, session_id: Uuid) -> Self {
        Self::new(user_id, role, session_id, Audience::App)
    }

    /// Like [`Actor::for_tests`], in the given audience (ADR-0095).
    pub fn for_tests_in(user_id: Uuid, role: Role, session_id: Uuid, audience: Audience) -> Self {
        Self::new(user_id, role, session_id, audience)
    }
}
