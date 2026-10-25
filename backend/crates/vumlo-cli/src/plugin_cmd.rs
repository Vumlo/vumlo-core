//! `vumlo-cli plugin ...` (ADR-0066): install, list, grant, disable, enable and remove the external
//! services that receive events and call the plugin API. Every command that changes the database
//! writes an audit row (no actor, source `cli`) in the same transaction, and says nothing was
//! changed, writing nothing, when it changes nothing. The token and the secret of an install are
//! printed once, by `install`, and nowhere else.

use std::io::{BufRead, Write};

use access::Scope;
use anyhow::{Context, bail};
use plugins::{Install, InstallStatus, PluginManifest, SecretKey};
use serde_json::json;
use sqlx::PgPool;
use vumlo_config::{Options, Settings};

use crate::runtime;

pub struct InstallArgs {
    pub base_url: String,
    /// Comma-separated scopes to grant, or `none`. Absent: ask for each scope that was requested.
    pub grant: Option<String>,
    pub dry_run: bool,
}

fn scope_list(scopes: &[Scope]) -> String {
    if scopes.is_empty() {
        "none".to_owned()
    } else {
        scopes
            .iter()
            .map(|scope| scope.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn scope_names(scopes: &[Scope]) -> Vec<&'static str> {
    scopes.iter().map(|scope| scope.as_str()).collect()
}

/// `a,b` or `none`.
fn parse_scopes(text: &str) -> anyhow::Result<Vec<Scope>> {
    if text.trim() == "none" {
        return Ok(Vec::new());
    }
    text.split(',')
        .map(|name| name.trim().parse::<Scope>().map_err(anyhow::Error::msg))
        .collect()
}

/// Asks, for each scope the plugin requested, whether to grant it.
fn ask(manifest: &PluginManifest) -> anyhow::Result<Vec<Scope>> {
    let mut granted = Vec::new();
    let stdin = std::io::stdin();
    for scope in &manifest.scopes {
        print!("grant `{}`? [y/N] ", scope.as_str());
        std::io::stdout()
            .flush()
            .context("cannot write the prompt")?;
        let mut line = String::new();
        stdin
            .lock()
            .read_line(&mut line)
            .context("cannot read the answer")?;
        if matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
            granted.push(*scope);
        }
    }
    Ok(granted)
}

async fn ready(settings: &Settings) -> anyhow::Result<PgPool> {
    let pool = runtime::connect(settings).await?;
    db::ensure_migrated_with(&pool, &runtime::migrators())
        .await
        .context("the database is not ready")?;
    Ok(pool)
}

fn key(settings: &Settings) -> anyhow::Result<SecretKey> {
    SecretKey::from_config(&settings.plugins.secret_key).context(
        "plugins.secret_key is not set: put VUMLO__PLUGINS__SECRET_KEY (32 characters or more, \
         for example from `openssl rand -base64 32`) in the environment",
    )
}

pub async fn install(options: &Options, args: InstallArgs) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let key = key(&loaded.settings)?;
    // The plugin is read before anything else: the answer decides what there is to approve.
    let manifest = plugins::fetch_manifest(&args.base_url).await?;
    println!("plugin {} {}", manifest.id, manifest.version);
    println!(
        "  subscribes to: {}",
        if manifest.subscribes.is_empty() {
            "nothing".to_owned()
        } else {
            manifest.subscribes.join(", ")
        }
    );
    println!("  asks for scopes: {}", scope_list(&manifest.scopes));
    let granted = match &args.grant {
        Some(text) => parse_scopes(text)?,
        None => ask(&manifest)?,
    };
    if let Some(extra) = granted.iter().find(|s| !manifest.scopes.contains(s)) {
        bail!(
            "`{}` cannot be granted: the plugin did not ask for it",
            extra.as_str()
        );
    }
    if args.dry_run {
        println!(
            "would install {} with the scopes: {}",
            manifest.id,
            scope_list(&granted)
        );
        return Ok(());
    }

    let pool = ready(&loaded.settings).await?;
    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    let installed = plugins::install(&mut tx, &key, &args.base_url, &manifest, &granted).await?;
    db::insert_cli_audit(
        &mut tx,
        "plugin.install",
        "plugin",
        installed.install.id,
        json!({
            "plugin_id": installed.install.plugin_id,
            "version": installed.install.version,
            "scopes": scope_names(&installed.install.scopes),
            "subscribes": installed.install.subscribes,
        }),
    )
    .await?;
    tx.commit().await.context("cannot commit")?;

    println!(
        "installed {} ({}) with the scopes: {}",
        installed.install.plugin_id,
        installed.install.id,
        scope_list(&installed.install.scopes)
    );
    println!(
        "These are shown once. Give them to the plugin, for example as environment variables:"
    );
    println!("  VUMLO_PLUGIN_TOKEN={}", installed.token.expose());
    println!("  VUMLO_WEBHOOK_SECRET={}", installed.secret);
    Ok(())
}

pub async fn list(options: &Options) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let pool = ready(&loaded.settings).await?;
    let mut conn = pool.acquire().await.context("cannot reach the database")?;
    let all = plugins::list(&mut conn).await?;
    if all.is_empty() {
        println!("no plugins installed");
    }
    for one in all {
        let install = &one.install;
        println!(
            "{:<16} {:<10} {:<9} scopes: {} (asked for: {}); deliveries: {} pending, {} failed",
            install.plugin_id,
            install.version,
            install.status.as_str(),
            scope_list(&install.scopes),
            scope_list(&install.requested_scopes),
            one.pending,
            one.failed,
        );
    }
    if loaded.settings.plugins.secret_key.expose().is_empty() {
        println!("warning: plugins.secret_key is not set, so no event is sent to any plugin");
    }
    Ok(())
}

pub async fn grant(
    options: &Options,
    plugin_id: &str,
    scopes: &str,
    dry_run: bool,
) -> anyhow::Result<()> {
    let wanted = parse_scopes(scopes)?;
    let loaded = runtime::load(options)?;
    let pool = ready(&loaded.settings).await?;
    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    let before = plugins::get(&mut tx, plugin_id).await?;
    if let Some(extra) = wanted.iter().find(|s| !before.requested_scopes.contains(s)) {
        bail!(
            "`{}` cannot be granted: {plugin_id} did not ask for it (it asked for: {})",
            extra.as_str(),
            scope_list(&before.requested_scopes)
        );
    }
    let mut sorted = wanted.clone();
    sorted.sort();
    sorted.dedup();
    let mut current = before.scopes.clone();
    current.sort();
    if sorted == current {
        println!(
            "{plugin_id} already has the scopes: {}; nothing was changed",
            scope_list(&before.scopes)
        );
        return Ok(());
    }
    if dry_run {
        println!(
            "would change the scopes of {plugin_id} from [{}] to [{}]",
            scope_list(&before.scopes),
            scope_list(&sorted)
        );
        return Ok(());
    }
    let after = plugins::grant(&mut tx, plugin_id, &wanted).await?;
    db::insert_cli_audit(
        &mut tx,
        "plugin.grant",
        "plugin",
        after.id,
        json!({
            "plugin_id": after.plugin_id,
            "from": scope_names(&before.scopes),
            "to": scope_names(&after.scopes),
        }),
    )
    .await?;
    tx.commit().await.context("cannot commit")?;
    println!(
        "{plugin_id} now has the scopes: {}",
        scope_list(&after.scopes)
    );
    Ok(())
}

/// `disable` and `enable`.
pub async fn set_status(
    options: &Options,
    plugin_id: &str,
    status: InstallStatus,
    dry_run: bool,
) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let pool = ready(&loaded.settings).await?;
    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    let before: Install = plugins::get(&mut tx, plugin_id).await?;
    if before.status == status {
        println!(
            "{plugin_id} is {} already; nothing was changed",
            status.as_str()
        );
        return Ok(());
    }
    if dry_run {
        println!("would make {plugin_id} {}", status.as_str());
        return Ok(());
    }
    let after = plugins::set_status(&mut tx, plugin_id, status).await?;
    let action = match status {
        InstallStatus::Disabled => "plugin.disable",
        InstallStatus::Enabled => "plugin.enable",
    };
    db::insert_cli_audit(
        &mut tx,
        action,
        "plugin",
        after.id,
        json!({"plugin_id": after.plugin_id}),
    )
    .await?;
    tx.commit().await.context("cannot commit")?;
    match status {
        InstallStatus::Disabled => {
            println!("{plugin_id} is disabled: its token stops working and it is sent no event")
        }
        InstallStatus::Enabled => println!("{plugin_id} is enabled"),
    }
    Ok(())
}

pub async fn remove(
    options: &Options,
    plugin_id: &str,
    yes: bool,
    dry_run: bool,
) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let pool = ready(&loaded.settings).await?;
    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    let before = plugins::get(&mut tx, plugin_id).await?;
    if dry_run {
        println!("would remove {plugin_id}, with the events it is owed and the reports it wrote");
        return Ok(());
    }
    if !yes {
        bail!(
            "removing {plugin_id} deletes the events it is owed and every report it wrote; \
             say --yes to do it (or --dry-run to look)"
        );
    }
    let removed = plugins::remove(&mut tx, plugin_id).await?;
    db::insert_cli_audit(
        &mut tx,
        "plugin.remove",
        "plugin",
        before.id,
        json!({"plugin_id": removed.plugin_id}),
    )
    .await?;
    tx.commit().await.context("cannot commit")?;
    println!("removed {plugin_id}");
    Ok(())
}
