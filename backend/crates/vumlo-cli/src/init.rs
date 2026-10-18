//! `vumlo-cli init`: the files a new installation starts from.

use std::io::Write;
use std::path::Path;

use anyhow::Context;

/// The commented defaults. The same file the repository ships and the tests load.
const CONFIG: &str = include_str!("../../../../deploy/vumlo.example.toml");
const ENV: &str = include_str!("../../../../deploy/env.example");

pub fn run(dir: &Path, dry_run: bool) -> anyhow::Result<()> {
    for (name, text) in [("vumlo.toml", CONFIG), (".env.example", ENV)] {
        let path = dir.join(name);
        if path.exists() {
            println!("skipped  {name}: already exists, not touched");
        } else if dry_run {
            println!("would create  {name}");
        } else {
            // `create_new` fails if the file appeared since the check: never overwrite.
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .with_context(|| format!("cannot create {}", path.display()))?;
            file.write_all(text.as_bytes())
                .with_context(|| format!("cannot write {}", path.display()))?;
            println!("created  {name}");
        }
    }
    println!();
    println!("Next: edit vumlo.toml, put the secrets in .env (copy .env.example, chmod 600),");
    println!("then run `vumlo-cli config check`.");
    Ok(())
}

#[cfg(test)]
mod tests;
