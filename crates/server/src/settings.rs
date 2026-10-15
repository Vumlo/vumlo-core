use std::net::SocketAddr;

use config::{Config, ConfigError, Environment, File, FileFormat, Source};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    pub server: ServerSettings,
    pub database: DatabaseSettings,
    pub log: LogSettings,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerSettings {
    pub bind_addr: SocketAddr,
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
    /// There are no defaults: every key must come from one of the two sources.
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
        Config::builder()
            .add_source(file)
            .add_source(env)
            .build()?
            .try_deserialize()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    const VALID: &str = r#"
        [server]
        bind_addr = "127.0.0.1:8080"
        [database]
        url = "postgres://u:p@localhost/db"
        max_connections = 5
        [log]
        format = "json"
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
}
