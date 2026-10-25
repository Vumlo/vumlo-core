//! What the crate talks about: the manifest a plugin publishes, an install, and the token.

use std::fmt;

use access::Scope;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use common::text_enum;
use rand::Rng;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// The event types a plugin may subscribe to. Anything else in a manifest stops the install.
pub const SUBSCRIBABLE: [&str; 2] = ["submission.submitted", "grade.published"];

/// The places in the web app a plugin may put a page (a later step of ADR-0066).
pub const SLOTS: [&str; 1] = [ASSIGNMENT_TAB];

/// A tab on the page of an assignment.
pub const ASSIGNMENT_TAB: &str = "assignment.tab";

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

/// What `GET {base}/manifest` says, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManifest {
    pub id: String,
    pub version: String,
    pub subscribes: Vec<String>,
    pub scopes: Vec<Scope>,
    pub ui: Vec<UiSlot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallStatus {
    Enabled,
    Disabled,
}
text_enum!(InstallStatus { Enabled => "enabled", Disabled => "disabled" });

/// One installed plugin. Holds no token and no secret: the token is stored only as a hash, and the
/// secret is derived when it is needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    pub id: Uuid,
    pub plugin_id: String,
    pub version: String,
    pub base_url: String,
    pub subscribes: Vec<String>,
    pub requested_scopes: Vec<Scope>,
    pub scopes: Vec<Scope>,
    /// The pages the plugin offers, as its manifest said (checked when it was installed).
    pub ui: Vec<UiSlot>,
    pub secret_version: i32,
    pub status: InstallStatus,
    pub created_at: DateTime<Utc>,
}

/// An install with the state of its deliveries, for `plugin list`.
#[derive(Debug, Clone, PartialEq, Eq)]
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
