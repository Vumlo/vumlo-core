//! The one schema of `vumlo.toml`, shared by the server and `vumlo-cli` (ADR-0054).
//!
//! Every key carries `x-visibility` in the generated JSON Schema: `public` (may reach the browser
//! through `client-config.json`), `private` (stays on the server) or `secret` (comes from the
//! environment only and is redacted when shown). A section marked `private` covers its keys; a
//! key can override its section. `schema::tests` fails on a key without a visibility.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::modules::{ModuleEntry, Modules};

/// The newest `config_version` this build understands.
pub const SUPPORTED_CONFIG_VERSION: u32 = 1;

/// Defines `as_str`, `FromStr` for a fieldless enum written as lowercase text.
macro_rules! text_enum {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = String;
            fn from_str(s: &str) -> Result<Self, String> {
                match s {
                    $($text => Ok(Self::$variant),)+
                    other => Err(format!(
                        "unknown {} `{other}` (expected one of: {})",
                        stringify!($name).to_lowercase(),
                        [$($text),+].join(", ")
                    )),
                }
            }
        }
    };
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct Settings {
    /// The layout of this file. Newer than the binary understands: refused.
    #[serde(default = "default_config_version")]
    #[schemars(extend("x-visibility" = "private"))]
    pub config_version: u32,
    pub server: ServerSettings,
    pub institution: InstitutionSettings,
    pub database: DatabaseSettings,
    #[serde(default)]
    pub web: WebSettings,
    #[serde(default)]
    pub cli: CliSettings,
    #[serde(default)]
    pub log: LogSettings,
    #[serde(default)]
    #[schemars(extend("x-visibility" = "private"))]
    pub auth: AuthSettings,
    #[serde(default)]
    #[schemars(extend("x-visibility" = "private"))]
    pub editor: EditorSettings,
    /// Outgoing mail (ADR-0045). Absent: the features that must send mail are off.
    #[serde(default)]
    #[schemars(extend("x-visibility" = "private"))]
    pub mail: Option<MailSettings>,
    /// Optional add-on modules: `name = true` or a `[modules.name]` table (ADR-0054).
    #[serde(default)]
    #[schemars(extend("x-visibility" = "private"))]
    pub modules: BTreeMap<String, ModuleEntry>,
    /// `modules` after the registry checked it. Filled by the loader.
    #[serde(skip)]
    #[schemars(skip)]
    pub module_state: Modules,
    /// The directory relative paths (`web.dir`, `cli.env_file`, `institution.logo`) are read from:
    /// the one the config file is in. Filled by the loader.
    #[serde(skip)]
    #[schemars(skip)]
    pub base_dir: std::path::PathBuf,
}

fn default_config_version() -> u32 {
    SUPPORTED_CONFIG_VERSION
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ServerSettings {
    #[serde(default = "default_bind_addr")]
    #[schemars(extend("x-visibility" = "private"))]
    pub bind_addr: SocketAddr,
    /// The origin browsers use to reach the API, e.g. `https://lms.example.edu`.
    /// Cookie-authenticated unsafe requests must carry exactly this `Origin`. Every URL the
    /// installation publishes is derived from it.
    #[schemars(extend("x-visibility" = "public"))]
    pub public_origin: String,
    /// Header that carries the client IP when a reverse proxy sets it (e.g. `x-real-ip`).
    /// Unset: the TCP peer address is used.
    #[serde(default)]
    #[schemars(extend("x-visibility" = "private"))]
    pub client_ip_header: Option<String>,
    /// Origins of trusted browser frontends hosted away from the API (ADR-0041), each exactly
    /// `scheme://host[:port]`. Empty: no CORS headers, and only `public_origin` passes the CSRF check.
    #[serde(default)]
    #[schemars(extend("x-visibility" = "private"))]
    pub allowed_origins: Vec<String>,
}

fn default_bind_addr() -> SocketAddr {
    ([127, 0, 0, 1], 8080).into()
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct DatabaseSettings {
    /// Secret: `DATABASE_URL` in the environment, never the file. Empty here means "not set";
    /// the loader refuses that.
    #[serde(default)]
    pub url: Secret,
    #[serde(default = "default_max_connections")]
    #[schemars(extend("x-visibility" = "private"))]
    pub max_connections: u32,
}

fn default_max_connections() -> u32 {
    10
}

/// Where the web app lives, for `vumlo-cli build web` and `doctor`.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct WebSettings {
    /// The web app's directory, relative to the config file.
    #[schemars(extend("x-visibility" = "private"))]
    pub dir: std::path::PathBuf,
}

impl Default for WebSettings {
    fn default() -> Self {
        Self { dir: "web".into() }
    }
}

/// What `vumlo-cli` itself needs.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct CliSettings {
    /// The file of `KEY=VALUE` lines the CLI reads the secrets from, relative to the config file.
    #[schemars(extend("x-visibility" = "private"))]
    pub env_file: std::path::PathBuf,
    /// Where `doctor` reaches the core. Default: `server.public_origin`.
    #[schemars(extend("x-visibility" = "private"))]
    pub core_url: Option<String>,
}

impl Default for CliSettings {
    fn default() -> Self {
        Self {
            env_file: ".env".into(),
            core_url: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct LogSettings {
    #[schemars(extend("x-visibility" = "private"))]
    pub format: LogFormat,
}

impl Default for LogSettings {
    fn default() -> Self {
        Self {
            format: LogFormat::Pretty,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Pretty,
    Json,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct AuthSettings {
    pub password_min_length: usize,
    pub cookie_secure: bool,
    /// The email domains that may sign up, by password (ADR-0046) or with Google (ADR-0047).
    /// Empty: nobody.
    pub allowed_domains: Vec<String>,
    /// Sign-in with Google (ADR-0047). Absent: the two endpoints are off.
    pub google: Option<GoogleSettings>,
    /// Login attempts one client may make in a burst. A client is an IPv4 address or an IPv6 /64
    /// (ADR-0038). A school behind one NAT raises this.
    pub login_ip_burst: u32,
    /// Seconds until a client gets one more login attempt back after a burst.
    pub login_ip_replenish_seconds: u64,
    /// Failed or in-flight login attempts per account in one window before the account is
    /// locked for the rest of it.
    pub login_account_max_attempts: u32,
    /// The length of that window, in minutes.
    pub login_account_window_minutes: u64,
    /// Password hashes and checks that may run at once. Each takes about 19 MiB.
    pub max_concurrent_hashes: usize,
}

impl Default for AuthSettings {
    fn default() -> Self {
        Self {
            password_min_length: 12,
            cookie_secure: true,
            allowed_domains: Vec::new(),
            google: None,
            login_ip_burst: 50,
            login_ip_replenish_seconds: 1,
            login_account_max_attempts: 10,
            login_account_window_minutes: 15,
            max_concurrent_hashes: 8,
        }
    }
}

/// The `[auth.google]` section (ADR-0047). Plain data: `Google::new` checks the two URLs when the
/// server starts.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GoogleSettings {
    pub client_id: String,
    /// Secret: `VUMLO__AUTH__GOOGLE__CLIENT_SECRET`.
    pub client_secret: Secret,
    /// Where Google sends the browser back: the callback page of the web app.
    pub redirect_uri: String,
    /// Default: Google. Another issuer is for a test server.
    #[serde(default = "default_google_issuer")]
    pub issuer: String,
}

fn default_google_issuer() -> String {
    "https://accounts.google.com".to_owned()
}

/// The `[institution]` section as it is loaded. `mode` has no default (ADR-0031).
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(try_from = "InstitutionSection")]
#[schemars(with = "InstitutionSection")]
pub struct InstitutionSettings {
    pub name: String,
    /// The name for narrow places such as the navbar; `name` when the key is absent (ADR-0044).
    pub short_name: String,
    /// The language the web app starts in (ADR-0043, ADR-0044).
    pub language: Language,
    /// The institution's IANA time zone, checked at startup (ADR-0044).
    #[serde(serialize_with = "serialize_time_zone")]
    pub time_zone: chrono_tz::Tz,
    /// Chosen before the first start and fixed for the life of the installation (ADR-0031).
    pub mode: InstitutionMode,
    /// The grading scale that applies: `institution.grading_scale` when set, else the one the
    /// mode implies (ADR-0022, ADR-0031).
    pub grading_scale: GradingScale,
    /// A path or an http(s) URL of the logo `vumlo-cli brand apply` builds the icons from.
    pub logo: Option<String>,
}

fn serialize_time_zone<S: serde::Serializer>(
    zone: &chrono_tz::Tz,
    s: S,
) -> Result<S::Ok, S::Error> {
    s.serialize_str(zone.name())
}

/// The keys of `[institution]` before the mode is checked and the scale is resolved. This
/// conversion is the only place that does either.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstitutionSection {
    #[schemars(extend("x-visibility" = "public"))]
    name: String,
    #[schemars(extend("x-visibility" = "public"))]
    short_name: Option<String>,
    #[schemars(extend("x-visibility" = "private"))]
    mode: Option<String>,
    #[schemars(extend("x-visibility" = "public"))]
    grading_scale: Option<GradingScale>,
    #[schemars(extend("x-visibility" = "public"))]
    language: Option<String>,
    #[schemars(extend("x-visibility" = "public"))]
    time_zone: Option<String>,
    #[schemars(extend("x-visibility" = "private"))]
    logo: Option<String>,
}

impl TryFrom<InstitutionSection> for InstitutionSettings {
    type Error = String;

    fn try_from(section: InstitutionSection) -> Result<Self, String> {
        let allowed = "set it to \"school\" or \"university\"; it is recorded at the first start \
                       and cannot be changed later";
        let mode = match section.mode {
            None => return Err(format!("institution.mode is required: {allowed}")),
            Some(text) => text
                .parse::<InstitutionMode>()
                .map_err(|_| format!("institution.mode \"{text}\" is not valid: {allowed}"))?,
        };
        let implied = match mode {
            InstitutionMode::School => GradingScale::Twelve,
            InstitutionMode::University => GradingScale::Hundred,
        };
        let short_name = match section.short_name {
            None => section.name.clone(),
            Some(text) => {
                let text = text.trim().to_owned();
                if !(1..=50).contains(&text.chars().count()) {
                    return Err("institution.short_name must be 1 to 50 characters long".to_owned());
                }
                text
            }
        };
        let language = match section.language {
            None => Language::En,
            Some(text) => text.parse().map_err(|_| {
                format!("institution.language \"{text}\" is not valid: set it to \"en\" or \"uk\"")
            })?,
        };
        let time_zone = match section.time_zone {
            None => chrono_tz::UTC,
            Some(text) => text.parse().map_err(|_| {
                format!(
                    "institution.time_zone \"{text}\" is not valid: set it to an IANA zone name \
                     such as \"Europe/Kyiv\" or \"UTC\""
                )
            })?,
        };
        let logo = section
            .logo
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty());
        Ok(Self {
            name: section.name,
            short_name,
            language,
            time_zone,
            mode,
            grading_scale: section.grading_scale.unwrap_or(implied),
            logo,
        })
    }
}

/// The interface language an institution starts in (ADR-0043, ADR-0044).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    En,
    Uk,
}

text_enum!(Language { En => "en", Uk => "uk" });

/// What kind of institution this installation serves (ADR-0002, ADR-0031).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum InstitutionMode {
    School,
    University,
}

text_enum!(InstitutionMode { School => "school", University => "university" });

/// How points relate to the grade (ADR-0022).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum GradingScale {
    /// The score of an assignment is its grade: at most 12 points per assignment.
    Twelve,
    /// The regular assignments of a course add up to at most 100 points.
    Hundred,
}

/// A value that must not reach a log or a printout: it shows as `***` in `Debug` (AGENTS section
/// 11) and as `<redacted>` when serialized.
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl Serialize for Secret {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("<redacted>")
    }
}

impl JsonSchema for Secret {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Secret".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"type": "string", "x-visibility": "secret"})
    }
}

/// The `[mail]` section (ADR-0045). Plain data: `Mailer::new` checks the address and the host when
/// the server starts.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MailSettings {
    /// The sender, `name@example.edu` or `Vumlo <no-reply@example.edu>`.
    pub from: String,
    /// The origin links in mails point to. Default: `server.public_origin`.
    #[serde(default)]
    pub link_base: Option<String>,
    pub transport: MailTransport,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum MailTransport {
    /// An SMTP relay.
    Smtp {
        host: String,
        /// Default: 465 for `tls`, 587 for `starttls`, 25 for `none`.
        #[serde(default)]
        port: Option<u16>,
        #[serde(default)]
        tls: SmtpTls,
        #[serde(default)]
        username: Option<String>,
        /// Secret: `VUMLO__MAIL__TRANSPORT__PASSWORD`.
        #[serde(default)]
        password: Option<Secret>,
    },
    /// One `.eml` file per message in `dir`: for development and tests.
    File { dir: std::path::PathBuf },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SmtpTls {
    /// Plain connection upgraded with STARTTLS (port 587).
    #[default]
    Starttls,
    /// TLS from the first byte (port 465).
    Tls,
    /// No encryption: a local test server such as Mailpit.
    #[serde(rename = "none")]
    Off,
}

/// What the document editor may offer (ADR-0025, ADR-0027). Plain data: `AppState::new` applies
/// it to the profiles in code (it can only narrow them) and rejects what does not fit.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct EditorSettings {
    /// Per profile name (`full`, `question`, `option`, `student`): the features it keeps.
    pub profiles: BTreeMap<String, ProfileSettings>,
    /// The providers a video block may name. Empty switches video off.
    pub video_providers: Vec<String>,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            profiles: BTreeMap::new(),
            video_providers: vec!["youtube".to_owned(), "vimeo".to_owned()],
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileSettings {
    pub features: Vec<String>,
}
