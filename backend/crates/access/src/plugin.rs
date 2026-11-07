//! Who a plugin is when it calls the plugin API (ADR-0066), and what it may do. The counterpart of
//! [`Actor`](crate::Actor): only the extractor in this module builds a [`PluginCaller`], from what
//! [`PluginSource`] resolved, and [`authorize_plugin`] is where scopes are decided.

use std::future::Future;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use common::{ApiError, text_enum};
use uuid::Uuid;

use crate::AccessError;

/// What an administrator may grant a plugin. A scope is the whole of what one kind of call may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    /// Read submitted attempts and their status, never a draft.
    SubmissionsRead,
    /// Store the plugin's own reports (ADR-0066, ADR-0077).
    ReportsWrite,
    /// Read the plugin's own settings, secrets included (ADR-0072).
    SettingsRead,
    /// Read the assignments a student can see, with the field definitions of a form (ADR-0074).
    AssignmentsRead,
    /// Read who is in a course and in which role: ids and names, no contact data (ADR-0074).
    RosterRead,
    /// Read published grades, never a draft (ADR-0074).
    GradesRead,
    /// Read the questions of a quiz without its answer key, and the attempts with their scores
    /// (ADR-0074).
    QuizzesRead,
    /// Read the email address of a member of a granted course (ADR-0074).
    UsersContact,
}
text_enum!(Scope {
    SubmissionsRead => "submissions:read",
    ReportsWrite => "reports:write",
    SettingsRead => "settings:read",
    AssignmentsRead => "assignments:read",
    RosterRead => "roster:read",
    GradesRead => "grades:read",
    QuizzesRead => "quizzes:read",
    UsersContact => "users:contact",
});

impl Scope {
    pub const ALL: [Scope; 8] = [
        Scope::SubmissionsRead,
        Scope::ReportsWrite,
        Scope::SettingsRead,
        Scope::AssignmentsRead,
        Scope::RosterRead,
        Scope::GradesRead,
        Scope::QuizzesRead,
        Scope::UsersContact,
    ];
}

/// The courses an administrator let a plugin reach (ADR-0074).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CourseScope {
    /// Every course, including the ones created later.
    All,
    /// Exactly these courses.
    Only(Vec<Uuid>),
}

impl CourseScope {
    pub fn allows(&self, course_id: Uuid) -> bool {
        match self {
            Self::All => true,
            Self::Only(courses) => courses.contains(&course_id),
        }
    }
}

/// An installed, enabled plugin whose token the request carried, with the scopes it was granted.
#[derive(Debug, Clone)]
pub struct PluginCaller {
    install_id: Uuid,
    plugin_id: String,
    scopes: Vec<Scope>,
    courses: CourseScope,
}

impl PluginCaller {
    pub(crate) fn new(
        install_id: Uuid,
        plugin_id: String,
        scopes: Vec<Scope>,
        courses: CourseScope,
    ) -> Self {
        Self {
            install_id,
            plugin_id,
            scopes,
            courses,
        }
    }

    pub fn install_id(&self) -> Uuid {
        self.install_id
    }

    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }
}

/// A request's token, resolved by whoever holds the installs.
#[derive(Debug, Clone)]
pub struct AuthenticatedPlugin {
    pub install_id: Uuid,
    pub plugin_id: String,
    pub scopes: Vec<Scope>,
    pub courses: CourseScope,
}

/// What the state of the application has to offer for a request to become a [`PluginCaller`]: it
/// finds the token, checks it and answers with the install. A missing, unknown or disabled plugin's
/// token is the error response, always `401`.
pub trait PluginSource: Send + Sync {
    fn authenticate_plugin(
        &self,
        parts: &Parts,
    ) -> impl Future<Output = Result<AuthenticatedPlugin, ApiError>> + Send;
}

impl<S: PluginSource> FromRequestParts<S> for PluginCaller {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let plugin = state.authenticate_plugin(parts).await?;
        Ok(Self::new(
            plugin.install_id,
            plugin.plugin_id,
            plugin.scopes,
            plugin.courses,
        ))
    }
}

/// Allows `scope` when the plugin was granted it. Deny by default: no scope, no call.
pub fn authorize_plugin(caller: &PluginCaller, scope: Scope) -> Result<(), AccessError> {
    if caller.scopes.contains(&scope) {
        Ok(())
    } else {
        Err(AccessError::Denied)
    }
}

/// Allows `scope` when the plugin was granted it and `course_id` is a course it may reach (ADR-0074).
/// Both are `Denied`: the plugin learns nothing from which one failed.
pub fn authorize_plugin_course(
    caller: &PluginCaller,
    scope: Scope,
    course_id: Uuid,
) -> Result<(), AccessError> {
    authorize_plugin(caller, scope)?;
    if caller.courses.allows(course_id) {
        Ok(())
    } else {
        Err(AccessError::Denied)
    }
}

/// The courses a list is about, for a plugin granted `scope`: the one it asked for, if it may reach
/// it (`Denied` otherwise), or, when it asked for none, every course it may reach. `None` means
/// every course there is.
pub fn plugin_course_filter(
    caller: &PluginCaller,
    scope: Scope,
    requested: Option<Uuid>,
) -> Result<Option<Vec<Uuid>>, AccessError> {
    authorize_plugin(caller, scope)?;
    match (requested, &caller.courses) {
        (Some(course), _) => {
            authorize_plugin_course(caller, scope, course)?;
            Ok(Some(vec![course]))
        }
        (None, CourseScope::All) => Ok(None),
        (None, CourseScope::Only(courses)) => Ok(Some(courses.clone())),
    }
}

#[cfg(feature = "testing")]
impl PluginCaller {
    /// A caller with these scopes and every course, for the tests of the crates above.
    pub fn for_tests(scopes: &[Scope]) -> Self {
        Self::for_install(Uuid::now_v7(), "test", scopes)
    }

    /// A caller that is the install `install_id`, for tests that go on to read its rows.
    pub fn for_install(install_id: Uuid, plugin_id: &str, scopes: &[Scope]) -> Self {
        Self::new(
            install_id,
            plugin_id.to_owned(),
            scopes.to_vec(),
            CourseScope::All,
        )
    }

    /// A caller that reaches only these courses.
    pub fn for_courses(scopes: &[Scope], courses: &[Uuid]) -> Self {
        Self::new(
            Uuid::now_v7(),
            "test".to_owned(),
            scopes.to_vec(),
            CourseScope::Only(courses.to_vec()),
        )
    }
}
