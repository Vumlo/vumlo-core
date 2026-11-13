use uuid::Uuid;

use common::{Role, text_enum};

/// Which door a session came through (ADR-0095). A session never changes audience. Administration
/// needs the console; everything else works from either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    /// The student app, and every bearer token.
    App,
    /// The admin console: its own origin, cookie and short session.
    Console,
}
text_enum!(Audience { App => "app", Console => "console" });

/// Who performs a request: an active user holding a valid session whose password is not
/// pending a forced change. Only the extractors of this module build one, from what
/// [`ActorSource`](super::ActorSource) resolved.
#[derive(Debug, Clone)]
pub struct Actor {
    user_id: Uuid,
    role: Role,
    session_id: Uuid,
    audience: Audience,
}

impl Actor {
    pub(crate) fn new(user_id: Uuid, role: Role, session_id: Uuid, audience: Audience) -> Self {
        Self {
            user_id,
            role,
            session_id,
            audience,
        }
    }

    pub fn user_id(&self) -> Uuid {
        self.user_id
    }

    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    /// The door the session came through. A handler uses it to clear the right cookie; whether
    /// the audience allows an action is `access::authorize`'s business.
    pub fn audience(&self) -> Audience {
        self.audience
    }

    /// Roles are read only by `policy`; everything else asks `access::authorize`.
    pub(crate) fn role(&self) -> Role {
        self.role
    }
}

/// Like [`Actor`], but the user may still owe a password change (`must_change_password`).
/// Allowed only on logout, `GET /me` and change password.
#[derive(Debug, Clone)]
pub struct PendingActor(pub Actor);
