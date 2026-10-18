//! `vumlo-cli doctor`: read-only diagnostics. It changes nothing, reports every check, and exits
//! non-zero if any failed. It carries on past a broken configuration or a database that is down,
//! so one run shows everything that is wrong.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, bail};
use api_types::{HealthResponse, ProblemDetails};
use module_api::{Check, CheckStatus, OutboundClient};
use serde_json::json;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use vumlo_config::{Loaded, Options, Settings};

use crate::runtime::{self, VERSION};

use CheckStatus::{Fail, Ok as Pass, Skip, Warn};

pub async fn run(options: &Options, as_json: bool) -> anyhow::Result<bool> {
    let mut checks = Vec::new();

    // Leniently: a module the config enables but this binary lacks is a warning here, not an
    // error, so the rest of the report still shows.
    let known: Vec<&str> = vumlo_config::known_modules()
        .iter()
        .map(|m| m.name)
        .collect();
    let loaded = vumlo_config::load_process(options, &known);
    let loaded = match loaded {
        Ok(loaded) => {
            checks.push(Check::new("configuration", Pass, describe(&loaded)));
            Some(loaded)
        }
        Err(e) => {
            checks.push(Check::new("configuration", Fail, e.to_string()));
            None
        }
    };
    checks.push(environment(options, loaded.as_ref()));
    checks.push(match &loaded {
        Some(l) => env_file(l),
        None => skip("env file", "needs a valid configuration"),
    });

    let pool = match &loaded {
        Some(l) => database(&l.settings, &mut checks).await,
        None => {
            checks.push(skip("database", "needs a valid configuration"));
            None
        }
    };
    match (&loaded, &pool) {
        (Some(_), Some(pool)) => checks.push(schema(pool).await),
        _ => checks.push(skip("schema", "needs the database")),
    }

    let health = match &loaded {
        Some(l) => core(&l.settings, &mut checks).await,
        None => {
            checks.push(skip("core", "needs a valid configuration"));
            None
        }
    };
    match &loaded {
        Some(l) => {
            checks.push(versions(&l.settings, health.as_ref()));
            checks.push(module_sets(health.as_ref()));
            checks.extend(modules(&l.settings, pool.as_ref()).await);
            checks.push(web_build(&l.settings));
        }
        None => {
            for name in ["versions", "module sets", "modules", "web build"] {
                checks.push(skip(name, "needs a valid configuration"));
            }
        }
    }
    checks.push(tool("node", "needed to build the web app"));
    checks.push(tool("docker", "only needed for the Docker install"));

    let ok = !checks.iter().any(|c| c.status == Fail);
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"ok": ok, "version": VERSION, "checks": checks}))?
        );
    } else {
        for check in &checks {
            let tag = match check.status {
                Pass => " ok ",
                Warn => "warn",
                Fail => "FAIL",
                Skip => "skip",
            };
            println!("[{tag}] {:<14} {}", check.name, check.detail);
        }
        println!();
        println!(
            "{}",
            if ok {
                "doctor: no failures"
            } else {
                "doctor: failed"
            }
        );
    }
    Ok(ok)
}

fn skip(name: &str, why: &str) -> Check {
    Check::new(name, Skip, why)
}

fn describe(loaded: &Loaded) -> String {
    match &loaded.config_file {
        Some(file) => format!("valid ({})", file.display()),
        None => "valid (no file: defaults, environment and flags)".to_owned(),
    }
}

/// `DATABASE_URL` is what the CLI cannot do without, whether or not the rest of the configuration
/// is valid. It may come from the environment or from the env file.
fn environment(options: &Options, loaded: Option<&Loaded>) -> Check {
    let mut vars: BTreeMap<String, String> = std::env::vars().collect();
    let env_file = match loaded {
        Some(l) => runtime::from_config_dir(&l.settings, &l.settings.cli.env_file),
        None => {
            let file = vumlo_config::find_file(options.config.as_deref(), &vars)
                .ok()
                .flatten();
            file.as_deref()
                .and_then(Path::parent)
                .filter(|p| !p.as_os_str().is_empty())
                .map_or_else(|| ".".into(), Path::to_path_buf)
                .join(".env")
        }
    };
    if let Ok(text) = std::fs::read_to_string(&env_file) {
        for (key, value) in vumlo_config::parse_env_file(&text) {
            vars.entry(key).or_insert(value);
        }
    }
    let present = ["DATABASE_URL", "VUMLO__DATABASE__URL"]
        .iter()
        .any(|name| vars.get(*name).is_some_and(|v| !v.is_empty()));
    if present {
        Check::new("environment", Pass, "DATABASE_URL is set")
    } else {
        Check::new(
            "environment",
            Fail,
            format!(
                "DATABASE_URL is not set: put it in the environment or in {}",
                env_file.display()
            ),
        )
    }
}

/// The env file holds the secrets: nobody but its owner may read it.
fn env_file(loaded: &Loaded) -> Check {
    let path = runtime::from_config_dir(&loaded.settings, &loaded.settings.cli.env_file);
    let Ok(meta) = std::fs::metadata(&path) else {
        return Check::new(
            "env file",
            Pass,
            format!(
                "{} not present; the secrets come from the environment",
                path.display()
            ),
        );
    };
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o077 == 0 {
        Check::new(
            "env file",
            Pass,
            format!("{} (mode {mode:o})", path.display()),
        )
    } else {
        Check::new(
            "env file",
            Fail,
            format!(
                "{} is readable by others (mode {mode:o}); run: chmod 600 {}",
                path.display(),
                path.display()
            ),
        )
    }
}

async fn database(settings: &Settings, checks: &mut Vec<Check>) -> Option<PgPool> {
    let connected = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(5))
        .connect(settings.database.url.expose())
        .await;
    let pool = match connected {
        Ok(pool) => pool,
        Err(e) => {
            checks.push(Check::new("database", Fail, format!("cannot connect: {e}")));
            return None;
        }
    };
    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&pool)
        .await
    {
        Ok(_) => {
            checks.push(Check::new("database", Pass, "reachable"));
            Some(pool)
        }
        Err(e) => {
            checks.push(Check::new(
                "database",
                Fail,
                format!("does not answer: {e}"),
            ));
            None
        }
    }
}

async fn schema(pool: &PgPool) -> Check {
    let mut conn = match pool.acquire().await {
        Ok(conn) => conn,
        Err(e) => return Check::new("schema", Fail, format!("cannot read it: {e}")),
    };
    let mut problems = Vec::new();
    let mut latest = None;
    let all = std::iter::once(("core", &db::MIGRATOR)).chain(runtime::migrators());
    for (label, migrator) in all {
        let state = match db::status(&mut conn, label, migrator).await {
            Ok(state) => state,
            Err(e) => return Check::new("schema", Fail, format!("cannot read it: {e}")),
        };
        if label == "core" {
            latest = state.applied.iter().max().copied();
        }
        if !state.newer.is_empty() {
            problems.push(format!(
                "the {label} schema is newer than this binary (migration(s) {:?} unknown to it)",
                state.newer
            ));
        }
        if let Some(version) = state.dirty {
            problems.push(format!("migration {version} of {label} did not finish"));
        }
        if !state.pending.is_empty() {
            problems.push(format!(
                "{} migration(s) of {label} pending; run `vumlo-cli db migrate`",
                state.pending.len()
            ));
        }
    }
    if problems.is_empty() {
        Check::new(
            "schema",
            Pass,
            format!(
                "up to date (core migration {})",
                latest.map_or("none".to_owned(), |v| v.to_string())
            ),
        )
    } else {
        Check::new("schema", Fail, problems.join("; "))
    }
}

async fn core(settings: &Settings, checks: &mut Vec<Check>) -> Option<HealthResponse> {
    let base = settings
        .cli
        .core_url
        .clone()
        .unwrap_or_else(|| settings.server.public_origin.clone());
    let url = format!("{}/api/v1/health", base.trim_end_matches('/'));
    let client =
        match OutboundClient::new(Duration::from_secs(3), Duration::from_secs(5), 64 * 1024, 0) {
            Ok(client) => client,
            Err(e) => {
                checks.push(Check::new("core", Fail, e.to_string()));
                return None;
            }
        };
    match client.get(&url).await {
        Err(e) => {
            checks.push(Check::new(
                "core",
                Fail,
                format!("not responding at {base}: {e}"),
            ));
            None
        }
        Ok((status, body)) => match parse_health(status, &body) {
            Ok(health) => {
                checks.push(Check::new(
                    "core",
                    Pass,
                    format!("responding at {base} (version {})", health.version),
                ));
                Some(health)
            }
            Err(e) => {
                checks.push(Check::new("core", Fail, format!("{e:#}")));
                None
            }
        },
    }
}

/// Turns a health response into `Ok` only when the server reports itself fully healthy.
fn parse_health(status: u16, body: &[u8]) -> anyhow::Result<HealthResponse> {
    if status != 200 {
        let Ok(p) = serde_json::from_slice::<ProblemDetails>(body) else {
            bail!("unexpected response status {status}");
        };
        bail!(
            "unhealthy: {} {}: {}",
            p.status,
            p.title,
            p.detail.unwrap_or_default()
        );
    }
    let health: HealthResponse =
        serde_json::from_slice(body).context("the server returned an invalid health response")?;
    if health.status != "ok" || health.database != "ok" {
        bail!(
            "unhealthy: status={}, database={}",
            health.status,
            health.database
        );
    }
    Ok(health)
}

/// The CLI, the server and the web app are one release: the same version tag.
fn versions(settings: &Settings, health: Option<&HealthResponse>) -> Check {
    let package = runtime::from_config_dir(settings, &settings.web.dir).join("package.json");
    let web = std::fs::read_to_string(&package)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| v.get("version").and_then(|v| v.as_str()).map(str::to_owned));
    let mut seen = vec![("vumlo-cli", VERSION.to_owned())];
    if let Some(health) = health {
        seen.push(("server", health.version.clone()));
    }
    if let Some(web) = web {
        seen.push(("web", web));
    }
    let line = seen
        .iter()
        .map(|(name, version)| format!("{name} {version}"))
        .collect::<Vec<_>>()
        .join(", ");
    if seen.iter().any(|(_, version)| *version != VERSION) {
        Check::new(
            "versions",
            Fail,
            format!("they must match, but: {line}; update everything to one release"),
        )
    } else if seen.len() < 3 {
        Check::new(
            "versions",
            Pass,
            format!("{line} (the rest could not be read)"),
        )
    } else {
        Check::new("versions", Pass, line)
    }
}

/// The CLI and the server must be built with the same Cargo features.
fn module_sets(health: Option<&HealthResponse>) -> Check {
    let Some(health) = health else {
        return skip("module sets", "needs the core to answer");
    };
    let mut cli: Vec<String> = runtime::compiled_names()
        .iter()
        .map(|n| (*n).to_owned())
        .collect();
    let mut server = health.modules.clone();
    cli.sort();
    server.sort();
    if cli == server {
        let shown = if cli.is_empty() {
            "none".to_owned()
        } else {
            cli.join(", ")
        };
        Check::new("module sets", Pass, format!("both built with: {shown}"))
    } else {
        Check::new(
            "module sets",
            Fail,
            format!(
                "vumlo-cli has [{}] but the server has [{}]; rebuild both with the same \
                 VUMLO_FEATURES",
                cli.join(", "),
                server.join(", ")
            ),
        )
    }
}

/// What the config says about modules against what this binary contains, and what the database
/// still holds of modules that are off or gone.
async fn modules(settings: &Settings, pool: Option<&PgPool>) -> Vec<Check> {
    let mut checks = Vec::new();
    let compiled = runtime::compiled();
    for module in vumlo_config::known_modules() {
        let enabled = settings.module_state.is_enabled(module.name);
        let built = compiled.iter().any(|m| m.name == module.name);
        if enabled && !built {
            checks.push(Check::new(
                format!("module {}", module.name),
                Warn,
                format!(
                    "enabled in the configuration, but this binary was built without it; build \
                     with the Cargo feature `{}`",
                    module.cargo_feature
                ),
            ));
        }
    }
    let Some(pool) = pool else {
        checks.push(skip("modules", "needs the database"));
        return checks;
    };
    if let Ok(mut conn) = pool.acquire().await
        && let Ok(migrated) = db::migrated_modules(&mut conn).await
    {
        for name in migrated {
            if !compiled.iter().any(|m| m.name == name) {
                checks.push(Check::new(
                    format!("module {name}"),
                    Warn,
                    "its data is stored in the database but this binary does not contain the \
                     module; nothing was deleted",
                ));
            }
        }
    }
    for module in compiled {
        let ctx = runtime::module_context(settings, pool, module.name);
        checks.extend((module.doctor)(ctx).await);
    }
    if checks.is_empty() {
        checks.push(Check::new("modules", Pass, "no optional modules"));
    }
    checks
}

/// The web build must be newer than the last `brand apply`: the build reads what it generated.
fn web_build(settings: &Settings) -> Check {
    let generated = settings
        .base_dir
        .join(".vumlo/generated/client-config.json");
    let build_id = runtime::from_config_dir(settings, &settings.web.dir).join(".next/BUILD_ID");
    let modified = |path: &Path| std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let Some(branded) = modified(&generated) else {
        return Check::new(
            "web build",
            Warn,
            "`vumlo-cli brand apply` has not been run (no .vumlo/generated/client-config.json)",
        );
    };
    let Some(built) = modified(&build_id) else {
        return Check::new(
            "web build",
            Warn,
            "the web app is not built; run `vumlo-cli build web`",
        );
    };
    if built >= branded {
        Check::new("web build", Pass, "built after the last brand apply")
    } else {
        Check::new(
            "web build",
            Warn,
            "brand apply ran after the last web build; run `vumlo-cli build web` and restart",
        )
    }
}

fn tool(name: &str, why: &str) -> Check {
    match Command::new(name).arg("--version").output() {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            Check::new(
                name,
                Pass,
                text.lines().next().unwrap_or_default().to_owned(),
            )
        }
        _ => Check::new(name, Warn, format!("not found ({why})")),
    }
}

#[cfg(test)]
mod tests;
