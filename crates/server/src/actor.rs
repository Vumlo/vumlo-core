use uuid::Uuid;

use crate::identity::Role;

/// Who performs a request: an active user holding a valid session whose password is not
/// pending a forced change. Only the `identity` extractors build one.
#[derive(Debug, Clone)]
pub struct Actor {
    user_id: Uuid,
    role: Role,
    session_id: Uuid,
}

impl Actor {
    pub(crate) fn new(user_id: Uuid, role: Role, session_id: Uuid) -> Self {
        Self {
            user_id,
            role,
            session_id,
        }
    }

    pub fn user_id(&self) -> Uuid {
        self.user_id
    }

    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    /// Roles are read only by `policy`; everything else asks `policy::authorize`.
    pub(crate) fn role(&self) -> Role {
        self.role
    }
}

/// Like [`Actor`], but the user may still owe a password change (`must_change_password`).
/// Allowed only on logout, `GET /me` and change password.
#[derive(Debug, Clone)]
pub struct PendingActor(pub Actor);
