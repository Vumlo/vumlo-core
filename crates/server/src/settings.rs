use std::collections::BTreeMap;
use std::net::SocketAddr;

use api_types::ProfileName;
use config::{Config, ConfigError, Environment, File, FileFormat, Source};
use serde::Deserialize;

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
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AuthSettings {
    pub password_min_length: usize,
    pub cookie_secure: bool,
    /// Stored now, used by self-registration (task 004).
    pub allowed_domains: Vec<String>,
}

impl Default for AuthSettings {
    fn default() -> Self {
        Self {
            password_min_length: 12,
            cookie_secure: true,
            allowed_domains: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstitutionSettings {
    pub name: String,
    /// Limits assignment points (ADR-0022). Optional; `institution.mode` does not exist yet.
    #[serde(default)]
    pub grading_scale: GradingScale,
}

/// How points relate to the grade (ADR-0022).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GradingScale {
    /// The score of an assignment is its grade: at most 12 points per assignment.
    Twelve,
    /// The regular assignments of a course add up to at most 100 points.
    #[default]
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
        let origin = self.server.public_origin.trim_end_matches('/');
        let host = origin
            .strip_prefix("https://")
            .or_else(|| origin.strip_prefix("http://"));
        if host.is_none_or(|h| h.is_empty() || h.contains(['/', '?', '#'])) {
            return Err(ConfigError::Message(
                "server.public_origin must look like https://host[:port], without a path"
                    .to_owned(),
            ));
        }
        self.server.public_origin = origin.to_owned();
        Ok(())
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
    fn grading_scale_defaults_to_hundred_and_parses_both_values() {
        assert_eq!(
            build(VALID, &[]).unwrap().institution.grading_scale,
            GradingScale::Hundred
        );
        for (text, scale) in [
            ("twelve", GradingScale::Twelve),
            ("hundred", GradingScale::Hundred),
        ] {
            let s = build(VALID, &[("LEARNLY__INSTITUTION__GRADING_SCALE", text)]).unwrap();
            assert_eq!(s.institution.grading_scale, scale);
        }
    }

    #[test]
    fn grading_scale_in_the_file_is_read() {
        let toml = VALID.replace(
            "name = \"Example University\"",
            "name = \"Example University\"\n        grading_scale = \"twelve\"",
        );
        assert_eq!(
            build(&toml, &[]).unwrap().institution.grading_scale,
            GradingScale::Twelve
        );
    }

    #[test]
    fn an_unknown_grading_scale_is_an_error_that_names_the_value() {
        let e = build(VALID, &[("LEARNLY__INSTITUTION__GRADING_SCALE", "ten")]).unwrap_err();
        assert!(e.to_string().contains("ten"), "{e}");
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
