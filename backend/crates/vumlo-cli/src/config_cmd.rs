//! `vumlo-cli config check` and `config show`.

use vumlo_config::{ClientConfig, Options};

use crate::runtime::{self, VERSION};

/// Validates the configuration: the file, the environment and the flags together, the way the
/// server would read them. Lists every problem; the exit code is non-zero if there is one.
pub fn check(options: &Options) -> anyhow::Result<bool> {
    match vumlo_config::load_process(options, &runtime::compiled_names()) {
        Ok(loaded) => {
            let s = &loaded.settings;
            if let Some(problem) = crate::brand::logo_problem(s) {
                eprintln!("configuration has 1 problem(s):");
                eprintln!("  - {problem}");
                return Ok(false);
            }
            println!("configuration OK");
            match &loaded.config_file {
                Some(file) => println!("  file         {}", file.display()),
                None => println!("  file         none (defaults, environment and flags only)"),
            }
            match &loaded.env_file {
                Some(file) => println!("  env file     {}", file.display()),
                None => println!("  env file     none"),
            }
            println!(
                "  institution  {} ({}, {})",
                s.institution.name,
                s.institution.mode.as_str(),
                s.institution.language.as_str()
            );
            println!("  public URL   {}", s.server.public_origin);
            let modules = s.module_state.statuses();
            if modules.is_empty() {
                println!("  modules      none in this build");
            } else {
                let line: Vec<String> = modules
                    .iter()
                    .map(|(name, status)| format!("{name}={}", status.as_str()))
                    .collect();
                println!("  modules      {}", line.join(", "));
            }
            Ok(true)
        }
        Err(e) => {
            eprintln!("configuration has {} problem(s):", e.0.len());
            for problem in &e.0 {
                eprintln!("  - {problem}");
            }
            Ok(false)
        }
    }
}

/// Prints the effective configuration as JSON. Secrets show as `<redacted>`; `--public` prints
/// exactly what `client-config.json` carries.
pub fn show(options: &Options, public: bool) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let settings = &loaded.settings;
    let value = if public {
        serde_json::to_value(ClientConfig::new(settings, VERSION, None))?
    } else {
        serde_json::to_value(settings)?
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
