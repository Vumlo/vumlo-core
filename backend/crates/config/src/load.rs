//! Reading the configuration: defaults < `vumlo.toml` < `VUMLO__*` environment < `--set` flags.
//!
//! Secrets come from the environment only. A secret in the file or in a flag, and a key the
//! schema does not know, are refused here, so the server and the CLI behave the same.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use api_types::ProfileName;
use config::{Config, Environment, File, FileFormat};
use serde_json::Value;

use crate::modules;
use crate::schema;
use crate::settings::{SUPPORTED_CONFIG_VERSION, Settings};

/// Every problem found, not only the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub Vec<String>);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.join("; "))
    }
}

impl std::error::Error for Error {}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Self(vec![message])
    }
}

pub enum FileSource {
    Path(PathBuf),
    Text(String),
}

/// Where the values come from, lowest priority first (the defaults are in the schema).
pub struct Sources {
    pub file: Option<FileSource>,
    /// The process environment. Only `VUMLO__*` and `DATABASE_URL` are configuration; module
    /// secrets are looked up here by name.
    pub env: BTreeMap<String, String>,
    /// `--set key=value`, highest priority.
    pub overrides: Vec<(String, String)>,
}

/// Keys whose value is a list: a comma-separated string in the environment and in `--set`.
fn list_keys() -> Vec<String> {
    let mut keys = vec![
        "auth.allowed_domains".to_owned(),
        "server.allowed_origins".to_owned(),
        "editor.video_providers".to_owned(),
    ];
    keys.extend(
        ProfileName::ALL
            .iter()
            .map(|p| format!("editor.profiles.{}.features", p.as_str())),
    );
    keys
}

/// The project used to be called Learnly. A config file or variable under the old name is not read
/// any more, so it is refused loudly instead of being ignored.
pub(crate) fn check_renamed(old_file: bool, new_file: bool, vars: &[String]) -> Result<(), Error> {
    let mut problems = Vec::new();
    if old_file && !new_file {
        problems.push("learnly.toml was found; rename it to vumlo.toml".to_owned());
    }
    for old in vars.iter().filter(|v| v.starts_with("LEARNLY")) {
        let new = old.replacen("LEARNLY", "VUMLO", 1);
        if !vars.contains(&new) {
            problems.push(format!("{old} was found; rename it to {new}"));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(Error(vec![format!(
            "the project is now called Vumlo: {}",
            problems.join("; ")
        )]))
    }
}

/// Builds the settings from `sources`. `compiled` names the optional modules this binary contains.
pub fn load(sources: Sources, compiled: &[&str]) -> Result<Settings, Error> {
    let names: Vec<String> = sources.env.keys().cloned().collect();
    check_renamed(false, false, &names)?;

    let mut errors = Vec::new();
    let secrets = schema::secret_paths();

    // 1. The file alone, to refuse secrets in it before they are merged with anything.
    let file_value = match &sources.file {
        None => Value::Object(serde_json::Map::new()),
        Some(source) => with_file(Config::builder(), source)
            .build()
            .and_then(Config::try_deserialize::<Value>)
            .map_err(|e| Error(vec![format!("cannot read the configuration file: {e}")]))?,
    };
    let mut refused = Vec::new();
    for path in leaf_paths(&file_value) {
        if let Some(why) = secret_problem(&path, &secrets, "vumlo.toml") {
            errors.push(why);
            refused.push(path);
        }
    }
    for (key, _) in &sources.overrides {
        let key = key.to_lowercase();
        if let Some(why) = secret_problem(&key, &secrets, "--set") {
            errors.push(why);
            refused.push(key);
        }
    }

    // 2. The merge.
    let mut builder = Config::builder();
    if let Some(source) = &sources.file {
        builder = with_file(builder, source);
    }
    builder = builder.add_source(environment(&sources.env));
    let lists = list_keys();
    for (key, value) in &sources.overrides {
        let key = key.to_lowercase();
        if refused.contains(&key) {
            continue;
        }
        builder = if lists.contains(&key) {
            let items: Vec<String> = value
                .split(',')
                .map(|item| item.trim().to_owned())
                .filter(|item| !item.is_empty())
                .collect();
            builder.set_override(key, items)
        } else if let Ok(flag) = value.parse::<bool>() {
            builder.set_override(key, flag)
        } else if let Ok(number) = value.parse::<i64>() {
            builder.set_override(key, number)
        } else {
            builder.set_override(key, value.as_str())
        }
        .map_err(|e| Error(vec![e.to_string()]))?;
    }
    let merged = builder.build().map_err(|e| Error(vec![e.to_string()]))?;
    let merged_value: Value = merged
        .clone()
        .try_deserialize()
        .map_err(|e| Error(vec![e.to_string()]))?;

    // 3. Keys the schema does not know, and the version.
    for path in schema::unknown_keys(&merged_value) {
        if !refused.contains(&path) {
            errors.push(unknown_message(&path));
        }
    }
    if let Some(version) = merged_value.get("config_version").and_then(Value::as_u64)
        && version > u64::from(SUPPORTED_CONFIG_VERSION)
    {
        errors.push(format!(
            "config_version {version} is newer than this binary supports \
             ({SUPPORTED_CONFIG_VERSION}); update vumlo"
        ));
    }
    if !errors.is_empty() {
        // The modules are checked on their own, so their problems show in the same report.
        let entries: Result<BTreeMap<String, modules::ModuleEntry>, _> =
            serde_json::from_value(merged_value.get("modules").cloned().unwrap_or(Value::Null));
        if let Ok(entries) = entries
            && let Err(found) = modules::resolve(&entries, &sources.env, compiled)
        {
            errors.extend(found);
        }
        return Err(Error(errors));
    }

    // 4. Typed settings, then the rules a type cannot hold.
    let mut settings: Settings = merged
        .try_deserialize()
        .map_err(|e| Error(vec![e.to_string()]))?;
    let mut problems = validate(&mut settings);
    match modules::resolve(&settings.modules, &sources.env, compiled) {
        Ok(state) => settings.module_state = state,
        Err(found) => problems.extend(found),
    }
    if problems.is_empty() {
        Ok(settings)
    } else {
        Err(Error(problems))
    }
}

fn with_file(
    builder: config::ConfigBuilder<config::builder::DefaultState>,
    source: &FileSource,
) -> config::ConfigBuilder<config::builder::DefaultState> {
    match source {
        FileSource::Path(path) => {
            builder.add_source(File::from(path.as_path()).format(FileFormat::Toml))
        }
        FileSource::Text(text) => builder.add_source(File::from_str(text, FileFormat::Toml)),
    }
}

fn environment(env: &BTreeMap<String, String>) -> Environment {
    let mut vars: std::collections::HashMap<String, String> = env
        .iter()
        .filter(|(name, _)| name.starts_with("VUMLO__"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if let Some(url) = env.get("DATABASE_URL") {
        vars.entry("VUMLO__DATABASE__URL".to_owned())
            .or_insert_with(|| url.clone());
    }
    let mut source = Environment::with_prefix("VUMLO")
        .prefix_separator("__")
        .separator("__")
        .try_parsing(true)
        .list_separator(",")
        .source(Some(vars));
    for key in list_keys() {
        source = source.with_list_parse_key(&key);
    }
    source
}

fn secret_problem(
    path: &str,
    secrets: &std::collections::BTreeSet<&str>,
    place: &str,
) -> Option<String> {
    if secrets.contains(path) {
        Some(format!(
            "{path} is a secret and must not be set in {place}: put it in the environment as {}",
            schema::env_name(path)
        ))
    } else if schema::looks_secret(path)
        && !schema::keys().iter().any(|k| k.path == path)
        && !is_module_key(path)
    {
        Some(format!(
            "{path} looks like a secret and must not be set in {place}: secrets come from the \
             environment only"
        ))
    } else {
        None
    }
}

/// `modules.<name>.<key>`: the registry gives those keys their own messages.
fn is_module_key(path: &str) -> bool {
    path.starts_with("modules.")
}

fn unknown_message(path: &str) -> String {
    let hint = if path == "institution.accent" {
        " (the accent colour is gone; the web app has one default)"
    } else {
        ""
    };
    format!("{path}: unknown key{hint}")
}

/// `a.b.c` for every leaf of a nested object; arrays are leaves.
fn leaf_paths(value: &Value) -> Vec<String> {
    fn walk(value: &Value, path: &str, out: &mut Vec<String>) {
        match value {
            Value::Object(map) if !map.is_empty() => {
                for (key, child) in map {
                    let next = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    walk(child, &next, out);
                }
            }
            _ if path.is_empty() => {}
            _ => out.push(path.to_owned()),
        }
    }
    let mut out = Vec::new();
    walk(value, "", &mut out);
    out
}

fn validate(settings: &mut Settings) -> Vec<String> {
    let mut errors = Vec::new();
    let auth = &settings.auth;
    if !(8..=128).contains(&auth.password_min_length) {
        errors.push("auth.password_min_length must be between 8 and 128".to_owned());
    }
    let ranges = [
        (
            "auth.login_ip_burst",
            u64::from(auth.login_ip_burst),
            1,
            1_000_000,
        ),
        (
            "auth.login_ip_replenish_seconds",
            auth.login_ip_replenish_seconds,
            1,
            3600,
        ),
        (
            "auth.login_account_max_attempts",
            u64::from(auth.login_account_max_attempts),
            1,
            1000,
        ),
        (
            "auth.login_account_window_minutes",
            auth.login_account_window_minutes,
            1,
            1440,
        ),
        (
            "auth.max_concurrent_hashes",
            u64::try_from(auth.max_concurrent_hashes).unwrap_or(u64::MAX),
            1,
            256,
        ),
    ];
    for (key, value, low, high) in ranges {
        if !(low..=high).contains(&value) {
            errors.push(format!("{key} must be between {low} and {high}"));
        }
    }
    for domain in &settings.auth.allowed_domains {
        if !is_domain(domain) {
            errors.push(format!(
                "auth.allowed_domains \"{domain}\" must be a lowercase domain name such as \
                 example.edu (no @, no spaces)"
            ));
        }
    }
    let mut check = |key: &str, value: &mut String| match origin(key, value) {
        Ok(clean) => *value = clean,
        Err(e) => errors.push(e),
    };
    check("server.public_origin", &mut settings.server.public_origin);
    for entry in &mut settings.server.allowed_origins {
        check("server.allowed_origins", entry);
    }
    if let Some(link_base) = settings.mail.as_mut().and_then(|m| m.link_base.as_mut()) {
        check("mail.link_base", link_base);
    }
    if let Some(core_url) = settings.cli.core_url.as_mut() {
        check("cli.core_url", core_url);
    }
    if let Some(scheme) = &settings.desktop.url_scheme
        && !crate::client::is_url_scheme(scheme)
    {
        errors.push(format!(
            "desktop.url_scheme \"{scheme}\" must be \"vumlo-\" and lowercase letters, digits and \
             hyphens, such as vumlo-ucu"
        ));
    }
    if settings.database.url.expose().is_empty() {
        errors.push(format!(
            "database.url is not set: put {} in the environment",
            schema::env_name("database.url")
        ));
    }
    errors
}

fn is_domain(text: &str) -> bool {
    let labels: Vec<&str> = text.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
}

/// An origin as a browser writes it in the `Origin` header (ADR-0041): `http` or `https`, a
/// lowercase host, a port that is not the scheme's default, nothing after that. One trailing `/`
/// is dropped. Anything else could never equal a real `Origin`, so it is refused at startup.
fn origin(key: &str, value: &str) -> Result<String, String> {
    let bad = || {
        format!(
            "{key} \"{value}\" must look like https://host[:port]: http or https, a lowercase \
             host, no path, no wildcard, no default port"
        )
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

// ---------------------------------------------------------------------------------------------
// The process: finding the file, the env file, flags
// ---------------------------------------------------------------------------------------------

/// What the command line may say about the configuration.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// `--config`.
    pub config: Option<PathBuf>,
    /// `--set key=value`, as typed.
    pub set: Vec<String>,
}

#[derive(Debug)]
pub struct Loaded {
    pub settings: Settings,
    /// The file the settings came from, if there was one.
    pub config_file: Option<PathBuf>,
    /// The env file whose variables were added to the environment, if it existed.
    pub env_file: Option<PathBuf>,
}

/// The configuration as the process sees it: `--config`, `VUMLO_CONFIG`, `./vumlo.toml`, then
/// `../vumlo.toml`; the variables of `[cli].env_file` (never overriding the environment); the
/// `VUMLO__*` variables; `--set`.
pub fn load_process(options: &Options, compiled: &[&str]) -> Result<Loaded, Error> {
    let mut env: BTreeMap<String, String> = std::env::vars().collect();
    let config_file = find_file(options.config.as_deref(), &env)?;
    let overrides = parse_overrides(&options.set)?;
    let base_dir = config_file
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."));
    let file = config_file.clone().map(FileSource::Path);

    let env_file = peek_env_file(&file, &env, &overrides, &base_dir);
    let mut read_env_file = None;
    if let Ok(text) = std::fs::read_to_string(&env_file) {
        for (key, value) in parse_env_file(&text) {
            env.entry(key).or_insert(value);
        }
        read_env_file = Some(env_file);
    }

    let mut settings = load(
        Sources {
            file,
            env,
            overrides,
        },
        compiled,
    )?;
    settings.base_dir = base_dir;
    Ok(Loaded {
        settings,
        config_file,
        env_file: read_env_file,
    })
}

/// `[cli].env_file` before the rest of the configuration is valid: the secrets it holds are what
/// makes the rest valid.
fn peek_env_file(
    file: &Option<FileSource>,
    env: &BTreeMap<String, String>,
    overrides: &[(String, String)],
    base_dir: &Path,
) -> PathBuf {
    let mut builder = Config::builder();
    if let Some(source) = file {
        builder = with_file(builder, source);
    }
    builder = builder.add_source(environment(env));
    let mut chosen = None;
    for (key, value) in overrides {
        if key.eq_ignore_ascii_case("cli.env_file") {
            chosen = Some(value.clone());
        }
    }
    let configured = chosen.or_else(|| {
        builder
            .build()
            .ok()
            .and_then(|c| c.get_string("cli.env_file").ok())
    });
    base_dir.join(configured.unwrap_or_else(|| ".env".to_owned()))
}

pub fn find_file(
    explicit: Option<&Path>,
    env: &BTreeMap<String, String>,
) -> Result<Option<PathBuf>, Error> {
    if let Some(path) = explicit {
        return if path.is_file() {
            Ok(Some(path.to_path_buf()))
        } else {
            Err(format!("--config {}: no such file", path.display()).into())
        };
    }
    if let Some(path) = env.get("VUMLO_CONFIG").filter(|p| !p.is_empty()) {
        let path = PathBuf::from(path);
        return if path.is_file() {
            Ok(Some(path))
        } else {
            Err(format!("VUMLO_CONFIG {}: no such file", path.display()).into())
        };
    }
    let new = ["vumlo.toml", "../vumlo.toml"].map(PathBuf::from);
    let found = new.iter().find(|p| p.is_file()).cloned();
    let old = ["learnly.toml", "../learnly.toml"]
        .iter()
        .any(|p| Path::new(p).is_file());
    check_renamed(old, found.is_some(), &[])?;
    Ok(found)
}

pub fn parse_overrides(set: &[String]) -> Result<Vec<(String, String)>, Error> {
    set.iter()
        .map(|item| {
            item.split_once('=')
                .map(|(k, v)| (k.trim().to_owned(), v.to_owned()))
                .filter(|(k, _)| !k.is_empty())
                .ok_or_else(|| Error(vec![format!("--set {item}: expected key=value")]))
        })
        .collect()
}

/// `KEY=VALUE` lines. `#` starts a comment line, `export ` is allowed, one pair of quotes around
/// the value is removed.
pub fn parse_env_file(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (key, value) = line.split_once('=')?;
            let value = value.trim();
            let value = ['"', '\'']
                .iter()
                .find_map(|q| value.strip_prefix(*q).and_then(|v| v.strip_suffix(*q)))
                .unwrap_or(value);
            Some((key.trim().to_owned(), value.to_owned()))
        })
        .collect()
}
