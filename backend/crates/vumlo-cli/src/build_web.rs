//! `vumlo-cli build web`: `brand apply`, then the web build as a subprocess.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};
use vumlo_config::Options;

use crate::{brand, runtime};

pub async fn run(options: &Options, dry_run: bool) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let settings = &loaded.settings;
    let web = runtime::from_config_dir(settings, &settings.web.dir);
    if !web.join("package.json").is_file() {
        bail!(
            "web.dir {} has no package.json: it must be the web/ directory of the repository",
            web.display()
        );
    }
    // Everything the build needs is checked before anything is written.
    let npm = Command::new("npm").arg("--version").output();
    if !npm.is_ok_and(|out| out.status.success()) {
        bail!(
            "npm was not found. Install Node 24 to build the web app here, or build it in Docker \
             (`docker compose build web`, see docs/install.md)"
        );
    }
    let generated = std::path::absolute(brand::generated_dir(settings))
        .context("cannot resolve the generated directory")?;
    let install = !web.join("node_modules").is_dir();

    brand::apply(options, dry_run).await?;
    if dry_run {
        if install {
            println!("would run  npm ci  in {}", web.display());
        }
        println!("would run  npm run build  in {}", web.display());
        return Ok(());
    }
    if install {
        let lock = web.join("package-lock.json").is_file();
        npm_in(&web, &generated, if lock { &["ci"] } else { &["install"] })?;
    }
    npm_in(&web, &generated, &["run", "build"])?;
    println!(
        "the web app is built in {}; restart the web service",
        web.display()
    );
    Ok(())
}

/// Runs npm in `web`, with the generated directory named in the environment (scripts/brand.mjs
/// reads it) and the output going straight to the terminal.
fn npm_in(web: &Path, generated: &Path, args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("npm")
        .args(args)
        .current_dir(web)
        .env("VUMLO_GENERATED_DIR", generated)
        .status()
        .context("cannot run npm")?;
    if !status.success() {
        bail!("`npm {}` failed ({status})", args.join(" "));
    }
    Ok(())
}
