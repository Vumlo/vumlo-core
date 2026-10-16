use std::net::SocketAddr;

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
        let mut settings: Self = Config::builder()
            .add_source(file)
            .add_source(
                env.try_parsing(true)
                    .list_separator(",")
                    .with_list_parse_key("auth.allowed_domains"),
            )
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
}
