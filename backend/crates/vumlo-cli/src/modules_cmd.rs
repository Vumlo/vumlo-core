//! `vumlo-cli modules list` and `vumlo-cli module <name> ...`.

use anyhow::{Context, bail};
use serde_json::json;
use vumlo_config::{ModuleStatus, Options};

use crate::runtime;

/// The modules every installation has (AGENTS section 4). They are always on and not configurable.
pub const CORE_MODULES: [&str; 9] = [
    "identity",
    "org",
    "teaching",
    "content",
    "assessment",
    "gradebook",
    "files",
    "events",
    "policy",
];

pub fn list(options: &Options, as_json: bool) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let optional = loaded.settings.module_state.statuses();
    if as_json {
        let optional: serde_json::Map<String, serde_json::Value> = optional
            .iter()
            .map(|(name, status)| (name.clone(), json!(status)))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"core": CORE_MODULES, "optional": optional}))?
        );
        return Ok(());
    }
    for name in CORE_MODULES {
        println!("{name:<16} core");
    }
    if optional.is_empty() {
        println!("(this build has no optional modules)");
    }
    for (name, status) in optional {
        let note = match status {
            ModuleStatus::Enabled => "enabled",
            ModuleStatus::Disabled => "disabled (compiled in, off)",
            ModuleStatus::Unavailable => "unavailable (not compiled in)",
        };
        println!("{name:<16} {note}");
    }
    Ok(())
}

/// Runs the subcommand a module brings. The module has to be compiled in and enabled.
pub async fn run_module(options: &Options, name: &str, args: &[String]) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let status = loaded.settings.module_state.status(name);
    let Some(module) = runtime::compiled().into_iter().find(|m| m.name == name) else {
        bail!(
            "module {name} is not compiled into this binary (status: {})",
            status.as_str()
        );
    };
    if status != ModuleStatus::Enabled {
        bail!(
            "module {name} is {}; enable it in vumlo.toml first",
            status.as_str()
        );
    }
    let Some(cli) = module.cli else {
        bail!("module {name} has no subcommands");
    };
    let matches = (cli.command)()
        .try_get_matches_from(std::iter::once(name.to_owned()).chain(args.iter().cloned()))
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let pool = runtime::connect(&loaded.settings).await?;
    db::ensure_migrated_with(&pool, &runtime::migrators())
        .await
        .context("the database is not ready")?;
    (cli.run)(
        matches,
        runtime::module_context(&loaded.settings, &pool, name),
    )
    .await
}
