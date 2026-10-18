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
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("vumlo-init-{name}-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn it_writes_both_files_once_and_never_overwrites() {
        let dir = scratch("twice");
        run(&dir, false).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("vumlo.toml")).unwrap(),
            CONFIG
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(".env.example")).unwrap(),
            ENV
        );

        std::fs::write(dir.join("vumlo.toml"), "mine").unwrap();
        run(&dir, false).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("vumlo.toml")).unwrap(),
            "mine"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_dry_run_writes_nothing() {
        let dir = scratch("dry");
        run(&dir, true).unwrap();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
