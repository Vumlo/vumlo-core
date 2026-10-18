//! `vumlo-cli db backup`: a `pg_dump` of the installation's database that is read back before the
//! command says it worked.
//!
//! The connection comes from `DATABASE_URL`, the one place the password lives. It reaches
//! `pg_dump` in its environment (`PGPASSWORD` and the rest), never in an argument, where every
//! user of the machine could read it with `ps`.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;

use anyhow::{Context, bail};
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use vumlo_config::Options;

use crate::runtime;

pub async fn run(options: &Options, output: Option<PathBuf>) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let env = libpq_env(loaded.settings.database.url.expose())?;
    let path = output.unwrap_or_else(|| {
        PathBuf::from(format!(
            "vumlo-{}.dump",
            chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
        ))
    });
    let shown = path.clone();
    let size = tokio::task::spawn_blocking(move || dump(&path, &env))
        .await
        .context("the backup task failed")??;
    println!(
        "backup: {} ({size} bytes, readable by pg_restore)",
        shown.display()
    );
    Ok(())
}

/// Dumps into `path` (a new file that only its owner can read), then lists the dump back.
fn dump(path: &Path, env: &[(&'static str, String)]) -> anyhow::Result<u64> {
    for tool in ["pg_dump", "pg_restore"] {
        let found = Command::new(tool)
            .arg("--version")
            .stdout(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !found {
            bail!("{tool} was not found: install the PostgreSQL client tools (postgresql-client)");
        }
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("cannot create {} (it must not exist yet)", path.display()))?
        .flush()?;
    let result = run_tools(path, env);
    if result.is_err() {
        // A failed dump leaves nothing that could be mistaken for a backup.
        let _ = std::fs::remove_file(path);
    }
    result?;
    Ok(std::fs::metadata(path)?.len())
}

fn run_tools(path: &Path, env: &[(&'static str, String)]) -> anyhow::Result<()> {
    let status = Command::new("pg_dump")
        .args(["--format=custom", "--file"])
        .arg(path)
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .status()
        .context("cannot run pg_dump")?;
    if !status.success() {
        bail!("pg_dump failed ({status})");
    }
    let listed = Command::new("pg_restore")
        .arg("--list")
        .arg(path)
        .stdout(Stdio::null())
        .status()
        .context("cannot run pg_restore")?;
    if !listed.success() || std::fs::metadata(path)?.len() == 0 {
        bail!("the dump does not read back with pg_restore ({listed}); it was removed");
    }
    Ok(())
}

/// The libpq environment variables for `url`.
fn libpq_env(url: &str) -> anyhow::Result<Vec<(&'static str, String)>> {
    if !(url.starts_with("postgres://") || url.starts_with("postgresql://")) {
        bail!("DATABASE_URL is not a PostgreSQL URL (postgres://user:password@host/database)");
    }
    let options =
        PgConnectOptions::from_str(url).context("DATABASE_URL is not a PostgreSQL URL")?;
    let host = match options.get_socket() {
        Some(socket) => socket.parent().map_or_else(
            || socket.display().to_string(),
            |dir| dir.display().to_string(),
        ),
        None => options.get_host().to_owned(),
    };
    let mut env = vec![
        ("PGHOST", host),
        ("PGPORT", options.get_port().to_string()),
        ("PGUSER", options.get_username().to_owned()),
        (
            "PGSSLMODE",
            match options.get_ssl_mode() {
                PgSslMode::Disable => "disable",
                PgSslMode::Allow => "allow",
                PgSslMode::Prefer => "prefer",
                PgSslMode::Require => "require",
                PgSslMode::VerifyCa => "verify-ca",
                PgSslMode::VerifyFull => "verify-full",
            }
            .to_owned(),
        ),
    ];
    if let Some(database) = options.get_database() {
        env.push(("PGDATABASE", database.to_owned()));
    }
    if let Some(password) = password(url) {
        env.push(("PGPASSWORD", password));
    }
    Ok(env)
}

/// The password of `postgres://user:password@host/db`, percent-decoded. sqlx does not give it back.
fn password(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?']).next()?;
    let userinfo = authority.rsplit_once('@')?.0;
    let (_, password) = userinfo.split_once(':')?;
    Some(percent_decode(password))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((high * 16 + low) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests;
