//! What the crate talks about: the manifest a plugin publishes, an install, and the token.

use std::collections::BTreeMap;
use std::fmt;

use access::{CourseRole, CourseScope, Scope};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use common::{Role, text_enum};
use rand::Rng;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::settings::SettingSpec;

/// The event types a plugin may subscribe to. Anything else in a manifest stops the install.
/// `plugin.settings.changed` is sent only to the plugin whose setting changed (ADR-0072). The events
/// that name a course (all but that one) are sent only to a plugin that may reach the course
/// (ADR-0074). A plugin gets only the events it subscribed to: nothing is sent by default.
pub const SUBSCRIBABLE: [&str; 9] = [
    "submission.submitted",
    "grade.published",
    SETTINGS_CHANGED,
    "assignment.created",
    "assignment.updated",
    "assignment.deadline_changed",
    "enrollment.changed",
    "quiz.attempt_submitted",
    "announcement.published",
];

/// The event type that says an administrator changed a setting of the plugin.
pub const SETTINGS_CHANGED: &str = "plugin.settings.changed";

/// The places in the web app a plugin may put a page (a later step of ADR-0066).
pub const SLOTS: [&str; 1] = [ASSIGNMENT_TAB];

/// A tab on the page of an assignment.
pub const ASSIGNMENT_TAB: &str = "assignment.tab";

/// What a plugin's report is about (ADR-0077).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportSubject {
    /// A submitted attempt.
    Submission(Uuid),
    Assignment(Uuid),
    Course(Uuid),
}

impl ReportSubject {
    pub fn id(self) -> Uuid {
        match self {
            Self::Submission(id) | Self::Assignment(id) | Self::Course(id) => id,
        }
    }
}

/// How many reports, each under its own key, one plugin keeps on one subject.
pub const MAX_REPORTS_PER_SUBJECT: i64 = 20;

/// The name a plugin gives a report: lowercase letters, digits, `_` and `-`, starting with a letter,
/// at most 40 characters.
pub fn is_report_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && key.len() <= 40
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// The version of the plugin protocol this core speaks.
pub const API_VERSION: &str = "1";

/// A page a plugin offers in one of the [`SLOTS`]. Stored with the install; used by the web app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiSlot {
    pub slot: String,
    pub title: String,
    /// Where the page is, relative to the plugin's base URL. Starts with `/`.
    pub path: String,
}

/// Who a route of the plugin lets in (ADR-0080).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteAuth {
    /// A person signed in to the core, and the roles the route lists. The default.
    Session,
    /// Anyone, with no session. The core checks nothing about the caller: the plugin checks its own
    /// token.
    None,
}
text_enum!(RouteAuth { Session => "session", None => "none" });

/// A route of the plugin's own server that people reach through the core (ADR-0078), as its manifest
/// declared it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteSpec {
    /// `GET`, `POST`, `PUT`, `PATCH` or `DELETE`; a public route is `GET` only.
    pub method: String,
    pub auth: RouteAuth,
    /// Starts with `/`; a segment is a name, or `{name}` for any one segment. `{course_id}` is a
    /// course the core resolves.
    pub path: String,
    /// The institution roles that may call it; none for a public route.
    pub roles: Vec<Role>,
    /// The roles in the course of `{course_id}` that may call it; empty: any.
    pub course_roles: Vec<CourseRole>,
}

/// The methods a route may declare.
pub const ROUTE_METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

/// The segment of a route path that names a course.
pub const COURSE_SEGMENT: &str = "course_id";

/// A literal segment, or the value of a parameter, is made of these and is not `.` or `..`: nothing a
/// plugin's server could read as a way up or out of the route that was checked.
pub fn is_plain_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'~' | b'-'))
}

impl RouteSpec {
    /// The values of the parameters when `path` (the part of the address after the plugin's id)
    /// is this route's, else `None`. Every segment of `path` is plain.
    pub fn matches<'p>(&self, path: &'p str) -> Option<Vec<(&str, &'p str)>> {
        let mut params = Vec::new();
        let mut wanted = self.path.split('/');
        let mut given = path.split('/');
        loop {
            match (wanted.next(), given.next()) {
                (None, None) => return Some(params),
                (Some(want), Some(have)) => {
                    // The path starts with `/`, so the first segment of both is empty.
                    if want.is_empty() && have.is_empty() {
                        continue;
                    }
                    if !is_plain_segment(have) {
                        return None;
                    }
                    match want.strip_prefix('{').and_then(|w| w.strip_suffix('}')) {
                        Some(name) => params.push((name, have)),
                        None if want == have => {}
                        None => return None,
                    }
                }
                _ => return None,
            }
        }
    }
}

/// What `GET {base}/manifest` says, checked.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginManifest {
    pub id: String,
    pub version: String,
    pub subscribes: Vec<String>,
    pub scopes: Vec<Scope>,
    pub ui: Vec<UiSlot>,
    /// What an administrator may configure, by key (checked).
    pub settings: BTreeMap<String, SettingSpec>,
    /// The routes of its own server people reach through the core (checked).
    pub routes: Vec<RouteSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallStatus {
    Enabled,
    Disabled,
}
text_enum!(InstallStatus { Enabled => "enabled", Disabled => "disabled" });

/// One installed plugin. Holds no token and no secret: the token is stored only as a hash, and the
/// secret is derived when it is needed.
#[derive(Debug, Clone, PartialEq)]
pub struct Install {
    pub id: Uuid,
    pub plugin_id: String,
    pub version: String,
    pub base_url: String,
    pub subscribes: Vec<String>,
    pub requested_scopes: Vec<Scope>,
    pub scopes: Vec<Scope>,
    /// The courses the plugin may reach (ADR-0074).
    pub courses: CourseScope,
    /// The pages the plugin offers, as its manifest said (checked when it was installed).
    pub ui: Vec<UiSlot>,
    /// The settings the plugin declared, as its manifest said (checked when it was installed).
    pub settings: BTreeMap<String, SettingSpec>,
    /// The routes the plugin declared (checked when it was installed).
    pub routes: Vec<RouteSpec>,
    pub secret_version: i32,
    pub status: InstallStatus,
    pub created_at: DateTime<Utc>,
}

/// An install with the state of its deliveries, for `plugin list`.
#[derive(Debug, Clone, PartialEq)]
pub struct InstallSummary {
    pub install: Install,
    pub pending: i64,
    pub failed: i64,
}

/// The result of an install: the token and the secret are shown once, here.
pub struct Installed {
    pub install: Install,
    pub token: PluginToken,
    pub secret: String,
}

impl fmt::Debug for Installed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Installed")
            .field("install", &self.install)
            .field("token", &"***")
            .field("secret", &"***")
            .finish()
    }
}

const TOKEN_PREFIX: &str = "vplg_";
/// 32 random bytes in unpadded base64url.
const TOKEN_BODY_LEN: usize = 43;

/// The credential of a plugin: `vplg_` and 32 random bytes. Only its SHA-256 is stored.
#[derive(Clone, PartialEq, Eq)]
pub struct PluginToken(String);

impl PluginToken {
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        Self(format!("{TOKEN_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes)))
    }

    /// Accepts only well-formed tokens, so junk never reaches the database.
    pub fn from_presented(raw: &str) -> Option<Self> {
        let body = raw.strip_prefix(TOKEN_PREFIX)?;
        let well_formed = body.len() == TOKEN_BODY_LEN
            && body
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        well_formed.then(|| Self(raw.to_owned()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn hash(&self) -> [u8; 32] {
        Sha256::digest(self.0.as_bytes()).into()
    }
}

impl fmt::Debug for PluginToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PluginToken(***)")
    }
}
