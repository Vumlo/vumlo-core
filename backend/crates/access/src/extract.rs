//! Turns a request into an `Actor`. Who a request belongs to is the business of whoever holds the
//! sessions; this module only asks (`ActorSource`) and builds the `Actor` from the answer, so the
//! one place that can build an `Actor` is here.

use std::future::Future;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use common::{ApiError, Role};
use uuid::Uuid;

use super::{AccessError, Actor, PendingActor};

/// A request's session, resolved by whoever holds the sessions.
#[derive(Debug, Clone)]
pub struct Authenticated {
    pub user_id: Uuid,
    pub role: Role,
    pub session_id: Uuid,
    /// The user signed in with a temporary password they still have to change.
    pub must_change_password: bool,
}

/// What the state of the application has to offer for a request to become an `Actor`: it finds
/// the credential, checks it and answers with the session. A missing or bad credential is the
/// error response.
pub trait ActorSource: Send + Sync {
    fn authenticate(
        &self,
        parts: &Parts,
    ) -> impl Future<Output = Result<Authenticated, ApiError>> + Send;
}

/// A user who may act: valid session, active account, no password change pending.
impl<S: ActorSource> FromRequestParts<S> for Actor {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let who = state.authenticate(parts).await?;
        if who.must_change_password {
            return Err(AccessError::PasswordChangeRequired.into());
        }
        Ok(Actor::new(who.user_id, who.role, who.session_id))
    }
}

/// Like `Actor`, but also accepts a user who still has to change a temporary password.
/// Only logout, `GET /me` and change password may use it.
impl<S: ActorSource> FromRequestParts<S> for PendingActor {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let who = state.authenticate(parts).await?;
        Ok(Self(Actor::new(who.user_id, who.role, who.session_id)))
    }
}
