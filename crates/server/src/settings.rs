use std::collections::BTreeMap;
use std::net::SocketAddr;

use api_types::ProfileName;
use config::{Config, ConfigError, Environment, File, FileFormat, Source};
use serde::Deserialize;

use crate::text_enum::text_enum;

#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    pub server: ServerSettings,
    pub database: DatabaseSettings,
    pub log: LogSettings,
    #[serde(default)]
    pub auth: AuthSettings,
    pub institution: InstitutionSettings,
    #[serde(default)]
    pub editor: EditorSettings,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerSettings {
    pub bind_addr: SocketAddr,
    /// The origin browsers use to reach the API, e.g. `https://lms.example.edu`.
    /// Cookie-authenticated unsafe requests must carry exactly this `Origin`.
    pub public_origin: String,
    /// Header that carries the client IP when a reverse proxy sets it (e.g. `x-real-ip`).
    /// Unset: the TCP peer address is used.
    #[serde(default)]
    pub client_ip_header: Option<String>,
    /// Origins of trusted browser frontends hosted away from the API (ADR-0041), each exactly
    /// `scheme://host[:port]`. Empty: no CORS headers, and only `public_origin` passes the CSRF check.
    #[serde(default)]
    pub allowed_origins: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AuthSettings {
    pub password_min_length: usize,
    pub cookie_secure: bool,
    /// Stored now, used by self-registration (task 004).
    pub allowed_domains: Vec<String>,
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
            login_ip_burst: 50,
            login_ip_replenish_seconds: 1,
            login_account_max_attempts: 10,
            login_account_window_minutes: 15,
            max_concurrent_hashes: 8,
        }
    }
}

/// The `[institution]` section as it is loaded. `mode` has no default (ADR-0031).
#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "InstitutionSection")]
pub struct InstitutionSettings {
    pub name: String,
    /// The name for narrow places such as the navbar; `name` when the key is absent (ADR-0044).
    pub short_name: String,
    /// The language the web app starts in (ADR-0043, ADR-0044).
    pub language: Language,
    /// The institution's IANA time zone, checked at startup (ADR-0044).
    pub time_zone: chrono_tz::Tz,
    pub accent: Accent,
    /// Chosen before the first start and fixed for the life of the installation (ADR-0031).
    pub mode: InstitutionMode,
    /// The grading scale that applies: `institution.grading_scale` when set, else the one the
    /// mode implies (ADR-0022, ADR-0031).
    pub grading_scale: GradingScale,
}

/// The keys of `[institution]` before the mode is checked and the scale is resolved. This
/// conversion is the only place that does either.
#[derive(Deserialize)]
struct InstitutionSection {
    name: String,
    short_name: Option<String>,
    mode: Option<String>,
    grading_scale: Option<GradingScale>,
    language: Option<String>,
    time_zone: Option<String>,
    accent: Option<String>,
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
        let accent = match section.accent {
            None => Accent::Indigo,
            Some(text) => text.parse().map_err(|_| {
                format!(
                    "institution.accent \"{text}\" is not valid: set it to \"indigo\", \"teal\", \
                     \"crimson\", \"forest\" or \"plum\""
                )
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
        Ok(Self {
            name: section.name,
            short_name,
            language,
            time_zone,
            accent,
            mode,
            grading_scale: section.grading_scale.unwrap_or(implied),
        })
    }
}

/// The interface language an institution starts in (ADR-0043, ADR-0044).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    En,
    Uk,
}

text_enum!(Language { En => "en", Uk => "uk" });

/// The accent colour of the web app. The values are the ones the web app has a palette for
/// (ADR-0044).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accent {
    Indigo,
    Teal,
    Crimson,
    Forest,
    Plum,
}

text_enum!(Accent {
    Indigo => "indigo",
    Teal => "teal",
    Crimson => "crimson",
    Forest => "forest",
    Plum => "plum",
});

/// What kind of institution this installation serves (ADR-0002, ADR-0031).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstitutionMode {
    School,
    University,
}

text_enum!(InstitutionMode { School => "school", University => "university" });

/// How points relate to the grade (ADR-0022).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GradingScale {
    /// The score of an assignment is its grade: at most 12 points per assignment.
    Twelve,
    /// The regular assignments of a course add up to at most 100 points.
    Hundred,
}

/// What the document editor may offer (ADR-0025, ADR-0027). Plain data: `AppState::new` applies
/// it to the profiles in code (it can only narrow them) and rejects what does not fit.
#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSettings {
    pub features: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseSettings {
    pub url: String,
    pub max_connections: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogSettings {
    pub format: LogFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Pretty,
    Json,
}

impl Settings {
    /// Reads the optional `learnly.toml`, then `LEARNLY__SECTION__KEY` environment overrides.
    /// Only the `[auth]` section has defaults; every other key must come from one of the two sources.
    pub fn load() -> Result<Self, ConfigError> {
        Self::build(
            File::with_name("learnly")
                .format(FileFormat::Toml)
                .required(false),
            Environment::with_prefix("LEARNLY").separator("__"),
        )
    }

    fn build(
        file: impl Source + Send + Sync + 'static,
        env: Environment,
    ) -> Result<Self, ConfigError> {
        let mut env = env
            .try_parsing(true)
            .list_separator(",")
            .with_list_parse_key("auth.allowed_domains")
            .with_list_parse_key("server.allowed_origins")
            .with_list_parse_key("editor.video_providers");
        for profile in ProfileName::ALL {
            env =
                env.with_list_parse_key(&format!("editor.profiles.{}.features", profile.as_str()));
        }
        let mut settings: Self = Config::builder()
            .add_source(file)
            .add_source(env)
            .build()?
            .try_deserialize()?;
        settings.validate()?;
        Ok(settings)
    }

    fn validate(&mut self) -> Result<(), ConfigError> {
        if !(8..=128).contains(&self.auth.password_min_length) {
            return Err(ConfigError::Message(
                "auth.password_min_length must be between 8 and 128".to_owned(),
            ));
        }
        let ranges = [
            (
                "auth.login_ip_burst",
                u64::from(self.auth.login_ip_burst),
                1,
                1_000_000,
            ),
            (
                "auth.login_ip_replenish_seconds",
                self.auth.login_ip_replenish_seconds,
                1,
                3600,
            ),
            (
                "auth.login_account_max_attempts",
                u64::from(self.auth.login_account_max_attempts),
                1,
                1000,
            ),
            (
                "auth.login_account_window_minutes",
                self.auth.login_account_window_minutes,
                1,
                1440,
            ),
            (
                "auth.max_concurrent_hashes",
                u64::try_from(self.auth.max_concurrent_hashes).unwrap_or(u64::MAX),
                1,
                256,
            ),
        ];
        for (key, value, low, high) in ranges {
            if !(low..=high).contains(&value) {
                return Err(ConfigError::Message(format!(
                    "{key} must be between {low} and {high}"
                )));
            }
        }
        self.server.public_origin = origin("server.public_origin", &self.server.public_origin)?;
        for entry in &mut self.server.allowed_origins {
            *entry = origin("server.allowed_origins", entry)?;
        }
        Ok(())
    }
}

/// An origin as a browser writes it in the `Origin` header (ADR-0041): `http` or `https`, a
/// lowercase host, a port that is not the scheme's default, nothing after that. One trailing `/`
/// is dropped. Anything else could never equal a real `Origin`, so it is refused at startup.
fn origin(key: &str, value: &str) -> Result<String, ConfigError> {
    let bad = || {
        ConfigError::Message(format!(
            "{key} \"{value}\" must look like https://host[:port]: http or https, a lowercase \
             host, no path, no wildcard, no default port"
        ))
    };
    let trimmed = value.strip_suffix('/').unwrap_or(value);
    let (default_port, rest) = if let Some(rest) = trimmed.strip_prefix("https://") {
        ("443", rest)
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        ("80", rest)
    } else {
        return Err(bad());
    };
    let (host, port) = if rest.starts_with('[') {
        match rest.split_once("]:") {
            Some((inner, port)) => (&rest[..=inner.len()], Some(port)),
            None => (rest, None),
        }
    } else {
        match rest.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (rest, None),
        }
    };
    let host_ok = if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        !inner.is_empty() && inner.chars().all(|c| c.is_ascii_hexdigit() || c == ':')
    } else {
        !host.is_empty()
            && !host.starts_with(['.', '-'])
            && !host.ends_with(['.', '-'])
            && host
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
    };
    let port_ok = port.is_none_or(|p| {
        p != default_port
            && !p.starts_with('0')
            && p.len() <= 5
            && p.chars().all(|c| c.is_ascii_digit())
            && p.parse::<u16>().is_ok()
    });
    if host_ok && port_ok {
        Ok(trimmed.to_owned())
    } else {
        Err(bad())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    const VALID: &str = r#"
        [server]
        bind_addr = "127.0.0.1:8080"
        public_origin = "https://lms.example.edu/"
        [database]
        url = "postgres://u:p@localhost/db"
        max_connections = 5
        [log]
        format = "json"
        [institution]
        name = "Example University"
        mode = "university"
    "#;

    fn build(toml: &str, env: &[(&str, &str)]) -> Result<Settings, ConfigError> {
        let env: HashMap<String, String> = env
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        Settings::build(
            File::from_str(toml, FileFormat::Toml),
            Environment::with_prefix("LEARNLY")
                .separator("__")
                .source(Some(env)),
        )
    }

    #[test]
    fn valid_file_loads() {
        let s = build(VALID, &[]).unwrap();
        assert_eq!(s.database.max_connections, 5);
        assert_eq!(s.log.format, LogFormat::Json);
    }

    #[test]
    fn environment_overrides_file() {
        let s = build(
            VALID,
            &[
                ("LEARNLY__DATABASE__URL", "postgres://other/db"),
                ("LEARNLY__LOG__FORMAT", "pretty"),
            ],
        )
        .unwrap();
        assert_eq!(s.database.url, "postgres://other/db");
        assert_eq!(s.log.format, LogFormat::Pretty);
    }

    #[test]
    fn invalid_log_format_is_an_error() {
        let e = build(VALID, &[("LEARNLY__LOG__FORMAT", "xml")]).unwrap_err();
        assert!(e.to_string().contains("xml"), "{e}");
    }

    #[test]
    fn invalid_bind_addr_is_an_error() {
        assert!(build(VALID, &[("LEARNLY__SERVER__BIND_ADDR", "nope")]).is_err());
    }

    #[test]
    fn missing_key_is_an_error() {
        let e = build("[log]\nformat = \"pretty\"", &[]).unwrap_err();
        assert!(e.to_string().contains("server"), "{e}");
    }

    #[test]
    fn auth_section_is_optional_and_defaults_are_safe() {
        let s = build(VALID, &[]).unwrap();
        assert_eq!(s.auth.password_min_length, 12);
        assert!(s.auth.cookie_secure);
        assert!(s.auth.allowed_domains.is_empty());
        assert_eq!(s.server.client_ip_header, None);
        assert_eq!(s.institution.name, "Example University");
    }

    #[test]
    fn auth_values_come_from_the_environment() {
        let s = build(
            VALID,
            &[
                ("LEARNLY__AUTH__PASSWORD_MIN_LENGTH", "16"),
                ("LEARNLY__AUTH__COOKIE_SECURE", "false"),
                ("LEARNLY__AUTH__ALLOWED_DOMAINS", "a.edu,b.edu"),
            ],
        )
        .unwrap();
        assert_eq!(s.auth.password_min_length, 16);
        assert!(!s.auth.cookie_secure);
        assert_eq!(s.auth.allowed_domains, ["a.edu", "b.edu"]);
    }

    #[test]
    fn password_min_length_must_be_between_8_and_128() {
        for bad in ["7", "129"] {
            let e = build(VALID, &[("LEARNLY__AUTH__PASSWORD_MIN_LENGTH", bad)]).unwrap_err();
            assert!(e.to_string().contains("password_min_length"), "{e}");
        }
        assert!(build(VALID, &[("LEARNLY__AUTH__PASSWORD_MIN_LENGTH", "8")]).is_ok());
        assert!(build(VALID, &[("LEARNLY__AUTH__PASSWORD_MIN_LENGTH", "128")]).is_ok());
    }

    #[test]
    fn login_limits_have_defaults_come_from_the_environment_and_are_bounded() {
        let defaults = build(VALID, &[]).unwrap().auth;
        assert_eq!(
            (
                defaults.login_ip_burst,
                defaults.login_ip_replenish_seconds,
                defaults.login_account_max_attempts,
                defaults.login_account_window_minutes,
                defaults.max_concurrent_hashes
            ),
            (50, 1, 10, 15, 8)
        );
        let raised = build(
            VALID,
            &[
                ("LEARNLY__AUTH__LOGIN_IP_BURST", "400"),
                ("LEARNLY__AUTH__MAX_CONCURRENT_HASHES", "2"),
            ],
        )
        .unwrap()
        .auth;
        assert_eq!(
            (raised.login_ip_burst, raised.max_concurrent_hashes),
            (400, 2)
        );
        for (key, bad) in [
            ("LOGIN_IP_BURST", "0"),
            ("LOGIN_IP_REPLENISH_SECONDS", "0"),
            ("LOGIN_ACCOUNT_MAX_ATTEMPTS", "0"),
            ("LOGIN_ACCOUNT_WINDOW_MINUTES", "1441"),
            ("MAX_CONCURRENT_HASHES", "0"),
        ] {
            let e = build(VALID, &[(&format!("LEARNLY__AUTH__{key}"), bad)]).unwrap_err();
            assert!(e.to_string().contains(&key.to_lowercase()), "{e}");
        }
    }

    #[test]
    fn grading_scale_follows_the_mode_and_parses_both_values() {
        for (mode, scale) in [
            ("school", GradingScale::Twelve),
            ("university", GradingScale::Hundred),
        ] {
            let s = build(VALID, &[("LEARNLY__INSTITUTION__MODE", mode)]).unwrap();
            assert_eq!(s.institution.grading_scale, scale, "{mode}");
        }
    }

    #[test]
    fn an_explicit_grading_scale_wins_over_the_mode() {
        for (mode, text, scale) in [
            ("school", "hundred", GradingScale::Hundred),
            ("university", "twelve", GradingScale::Twelve),
        ] {
            let s = build(
                VALID,
                &[
                    ("LEARNLY__INSTITUTION__MODE", mode),
                    ("LEARNLY__INSTITUTION__GRADING_SCALE", text),
                ],
            )
            .unwrap();
            assert_eq!(s.institution.grading_scale, scale, "{mode} + {text}");
        }
    }

    #[test]
    fn grading_scale_in_the_file_is_read() {
        let toml = VALID.replace(
            "mode = \"university\"",
            "mode = \"university\"\n        grading_scale = \"twelve\"",
        );
        assert_eq!(
            build(&toml, &[]).unwrap().institution.grading_scale,
            GradingScale::Twelve
        );
    }

    #[test]
    fn a_missing_mode_is_an_error_that_names_the_key_and_the_values() {
        let toml = VALID.replace("mode = \"university\"", "");
        let e = build(&toml, &[]).unwrap_err().to_string();
        for needle in ["institution.mode", "school", "university"] {
            assert!(e.contains(needle), "{needle}: {e}");
        }
    }

    #[test]
    fn an_unknown_mode_is_an_error_that_names_the_key_and_the_value() {
        let e = build(VALID, &[("LEARNLY__INSTITUTION__MODE", "college")])
            .unwrap_err()
            .to_string();
        for needle in ["institution.mode", "college", "school", "university"] {
            assert!(e.contains(needle), "{needle}: {e}");
        }
    }

    #[test]
    fn both_modes_parse_and_the_environment_overrides_the_file() {
        assert_eq!(
            build(VALID, &[]).unwrap().institution.mode,
            InstitutionMode::University
        );
        let s = build(VALID, &[("LEARNLY__INSTITUTION__MODE", "school")]).unwrap();
        assert_eq!(s.institution.mode, InstitutionMode::School);
    }

    #[test]
    fn the_mode_alone_is_enough_without_a_file_value() {
        let toml = VALID.replace("mode = \"university\"", "");
        let s = build(&toml, &[("LEARNLY__INSTITUTION__MODE", "school")]).unwrap();
        assert_eq!(s.institution.mode, InstitutionMode::School);
    }

    #[test]
    fn an_unknown_grading_scale_is_an_error_that_names_the_value() {
        let e = build(VALID, &[("LEARNLY__INSTITUTION__GRADING_SCALE", "ten")]).unwrap_err();
        assert!(e.to_string().contains("ten"), "{e}");
    }

    #[test]
    fn the_institution_profile_has_defaults() {
        let i = build(VALID, &[]).unwrap().institution;
        assert_eq!(i.short_name, "Example University");
        assert_eq!(i.language, Language::En);
        assert_eq!(i.time_zone, chrono_tz::UTC);
        assert_eq!(i.accent, Accent::Indigo);
    }

    #[test]
    fn the_institution_profile_is_read_from_the_file_and_the_environment() {
        let toml = VALID.replace(
            "mode = \"university\"",
            "mode = \"university\"\n        short_name = \" Example \"\n        language = \"uk\"\n        \
             time_zone = \"Europe/Kyiv\"\n        accent = \"teal\"",
        );
        let i = build(&toml, &[]).unwrap().institution;
        assert_eq!(i.short_name, "Example");
        assert_eq!(i.language, Language::Uk);
        assert_eq!(i.time_zone, chrono_tz::Europe::Kyiv);
        assert_eq!(i.accent, Accent::Teal);
        let i = build(&toml, &[("LEARNLY__INSTITUTION__ACCENT", "plum")])
            .unwrap()
            .institution;
        assert_eq!(i.accent, Accent::Plum);
    }

    #[test]
    fn a_bad_institution_profile_value_stops_the_server_and_names_the_key_and_the_value() {
        for (key, bad) in [
            ("SHORT_NAME", ""),
            ("SHORT_NAME", &"x".repeat(51)),
            ("LANGUAGE", "fr"),
            ("TIME_ZONE", "Europe/Kyev"),
            ("TIME_ZONE", "+03:00"),
            ("ACCENT", "#ff0000"),
        ] {
            let e = build(VALID, &[(&format!("LEARNLY__INSTITUTION__{key}"), bad)])
                .unwrap_err()
                .to_string();
            let needle = format!("institution.{}", key.to_lowercase());
            assert!(e.contains(&needle), "{key}={bad}: {e}");
        }
        let e = build(VALID, &[("LEARNLY__INSTITUTION__TIME_ZONE", "Europe/Kyev")])
            .unwrap_err()
            .to_string();
        assert!(e.contains("Europe/Kyev"), "{e}");
    }

    #[test]
    fn public_origin_loses_the_trailing_slash_and_rejects_paths() {
        assert_eq!(
            build(VALID, &[]).unwrap().server.public_origin,
            "https://lms.example.edu"
        );
        for bad in ["lms.example.edu", "https://lms.example.edu/app", "ftp://x"] {
            let e = build(VALID, &[("LEARNLY__SERVER__PUBLIC_ORIGIN", bad)]).unwrap_err();
            assert!(e.to_string().contains("public_origin"), "{bad}: {e}");
        }
    }

    #[test]
    fn allowed_origins_default_to_none_and_come_from_a_list() {
        assert!(build(VALID, &[]).unwrap().server.allowed_origins.is_empty());
        let settings = build(
            VALID,
            &[(
                "LEARNLY__SERVER__ALLOWED_ORIGINS",
                "https://app.example.edu/,http://localhost:5173,https://[::1]:8443",
            )],
        )
        .unwrap();
        assert_eq!(
            settings.server.allowed_origins,
            [
                "https://app.example.edu",
                "http://localhost:5173",
                "https://[::1]:8443"
            ]
        );
        let toml = VALID.replace(
            "public_origin = ",
            "allowed_origins = [\"https://a.test\"]\npublic_origin = ",
        );
        assert_eq!(
            build(&toml, &[]).unwrap().server.allowed_origins,
            ["https://a.test"]
        );
    }

    #[test]
    fn invalid_origins_stop_the_server_and_name_the_key() {
        for bad in [
            "*",
            "https://*.example.edu",
            "null",
            "",
            "app.example.edu",
            "ftp://a.test",
            "https://a.test/x",
            "https://a.test?q=1",
            "https://a.test#f",
            "https://A.test",
            "https://a.test:443",
            "http://a.test:80",
            "https://a.test:0",
            "https://a.test:99999",
            "https://a.test:",
            "https://a.test:80a",
            "https://user@a.test",
            "https://a.test//",
            "https://",
            "https://[]",
        ] {
            let e = build(VALID, &[("LEARNLY__SERVER__ALLOWED_ORIGINS", bad)]).unwrap_err();
            assert!(
                e.to_string().contains("server.allowed_origins"),
                "{bad:?}: {e}"
            );
            let e = build(VALID, &[("LEARNLY__SERVER__PUBLIC_ORIGIN", bad)]).unwrap_err();
            assert!(
                e.to_string().contains("server.public_origin"),
                "{bad:?}: {e}"
            );
        }
        // One bad entry among good ones is still refused.
        let e = build(
            VALID,
            &[("LEARNLY__SERVER__ALLOWED_ORIGINS", "https://a.test,*")],
        )
        .unwrap_err();
        assert!(e.to_string().contains("server.allowed_origins"), "{e}");
    }

    fn with_editor(extra: &str) -> String {
        format!("{VALID}\n[editor]\n{extra}")
    }

    #[test]
    fn the_editor_section_is_optional_and_video_is_on_by_default() {
        let s = build(VALID, &[]).unwrap();
        assert!(s.editor.profiles.is_empty());
        assert_eq!(s.editor.video_providers, ["youtube", "vimeo"]);
    }

    #[test]
    fn editor_values_come_from_the_file_and_the_environment() {
        let toml = with_editor(
            "video_providers = [\"vimeo\"]\n[editor.profiles.option]\nfeatures = [\"links\"]",
        );
        let s = build(&toml, &[]).unwrap();
        assert_eq!(s.editor.video_providers, ["vimeo"]);
        assert_eq!(s.editor.profiles["option"].features, ["links"]);

        let s = build(
            VALID,
            &[
                ("LEARNLY__EDITOR__VIDEO_PROVIDERS", "youtube"),
                (
                    "LEARNLY__EDITOR__PROFILES__STUDENT__FEATURES",
                    "text_styles,links",
                ),
            ],
        )
        .unwrap();
        assert_eq!(s.editor.video_providers, ["youtube"]);
        assert_eq!(
            s.editor.profiles["student"].features,
            ["text_styles", "links"]
        );
    }

    #[test]
    fn an_empty_provider_list_is_allowed_and_switches_video_off() {
        let s = build(&with_editor("video_providers = []"), &[]).unwrap();
        assert!(s.editor.video_providers.is_empty());
    }

    #[test]
    fn an_unknown_key_in_an_editor_profile_is_a_parse_error_that_names_it() {
        let toml = with_editor("[editor.profiles.full]\nfeatures = []\ncolor = \"red\"");
        let e = build(&toml, &[]).unwrap_err();
        assert!(e.to_string().contains("color"), "{e}");
    }

    #[test]
    fn settings_hold_the_editor_section_as_data_and_what_it_means_is_checked_later() {
        // `AppState::new` resolves the profiles and rejects these (tests/api.rs).
        for extra in [
            "[editor.profiles.wizard]\nfeatures = []",
            "[editor.profiles.full]\nfeatures = [\"telepathy\"]",
            "[editor.profiles.option]\nfeatures = [\"headings\"]",
            "video_providers = [\"dailymotion\"]",
        ] {
            build(&with_editor(extra), &[]).unwrap();
        }
        let s = build(VALID, &[("LEARNLY__EDITOR__VIDEO_PROVIDERS", "vimeo,nope")]).unwrap();
        assert_eq!(s.editor.video_providers, ["vimeo", "nope"]);
    }
}
