//! The only place that decides who may do what (ADR-0004). Deny by default.

use crate::actor::Actor;
use crate::identity::Role;

/// What an actor wants to do. The resource a variant needs travels inside it, so course-level
/// actions can be added as new variants without touching existing callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Read or change one's own profile, password and sessions; log out.
    ManageOwnAccount,
    /// Create, list, change, disable and reset users, institution-wide.
    ManageUsers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not allowed")]
pub struct PolicyError;

/// Allows or denies `action`. The match has no wildcard: a new action does not compile
/// until someone decides who may do it.
pub fn authorize(actor: &Actor, action: Action) -> Result<(), PolicyError> {
    let allowed = match action {
        Action::ManageOwnAccount => true,
        Action::ManageUsers => actor.role() == Role::Admin,
    };
    if allowed { Ok(()) } else { Err(PolicyError) }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    fn actor(role: Role) -> Actor {
        Actor::new(Uuid::now_v7(), role, Uuid::now_v7())
    }

    #[test]
    fn every_role_may_manage_their_own_account() {
        for role in [Role::Admin, Role::Teacher, Role::Student] {
            assert_eq!(authorize(&actor(role), Action::ManageOwnAccount), Ok(()));
        }
    }

    #[test]
    fn only_admins_manage_users() {
        assert_eq!(authorize(&actor(Role::Admin), Action::ManageUsers), Ok(()));
        for role in [Role::Teacher, Role::Student] {
            assert_eq!(
                authorize(&actor(role), Action::ManageUsers),
                Err(PolicyError),
                "{role:?}"
            );
        }
    }
}
