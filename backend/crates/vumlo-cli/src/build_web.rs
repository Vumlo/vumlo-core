//! `vumlo-cli build web`: `brand apply`, then the web build as a subprocess, and the admin console's (ADR-0095)
//! when the release has one beside it.

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
    let install = !web.join("node_modules").is_dir();
    // The admin console is a static app of its own, next to web/ in the release; it reads the same branding.
    let console = web
        .parent()
        .map(|dir| dir.join("web-admin"))
        .filter(|dir| dir.join("package.json").is_file());

    brand::apply(options, dry_run)?;
    if dry_run {
        if install {
            println!("would run  npm ci  in {}", web.display());
        }
        println!("would run  npm run build  in {}", web.display());
        if let Some(console) = &console {
            if !console.join("node_modules").is_dir() {
                println!("would run  npm ci  in {}", console.display());
            }
            println!("would run  npm run build  in {}", console.display());
        }
        return Ok(());
    }
    if install {
        let lock = web.join("package-lock.json").is_file();
        npm_in(&web, if lock { &["ci"] } else { &["install"] })?;
    }
    npm_in(&web, &["run", "build"])?;
    println!(
        "the web app is built in {}; restart the web service",
        web.display()
    );
    if let Some(console) = console {
        if !console.join("node_modules").is_dir() {
            let lock = console.join("package-lock.json").is_file();
            npm_in(&console, if lock { &["ci"] } else { &["install"] })?;
        }
        npm_in(&console, &["run", "build"])?;
        println!(
            "the admin console is built in {}; its files are served as they are",
            console.join("dist").display()
        );
    }
    Ok(())
}

/// Runs npm in `web`, the output going straight to the terminal.
fn npm_in(web: &Path, args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("npm")
        .args(args)
        .current_dir(web)
        .status()
        .context("cannot run npm")?;
    if !status.success() {
        bail!("`npm {}` failed ({status})", args.join(" "));
    }
    Ok(())
}
