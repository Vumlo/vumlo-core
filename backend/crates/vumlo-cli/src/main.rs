//! `vumlo-cli`: the administrator CLI of Vumlo (ADR-0054).
//!
//! A separate binary and process from the server, shipped with it in the same release. It runs
//! where the configuration and the database are and talks to PostgreSQL directly, so every
//! command that changes the database writes `events.audit_log` (no actor, source `cli`). It
//! prepares configuration, generated assets and the database; it does not deploy anything.

mod admin;
mod config_cmd;
mod db_cmd;
mod doctor;
mod init;
mod modules_cmd;
mod runtime;
mod version;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use vumlo_config::Options;

#[derive(Parser)]
#[command(name = "vumlo-cli", version, about = "The administrator CLI of Vumlo")]
struct Cli {
    /// The configuration file. Default: $VUMLO_CONFIG, ./vumlo.toml, then ../vumlo.toml.
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Override one key, e.g. `--set server.public_origin=https://lms.example.edu`. The highest
    /// priority of all; secrets cannot be set this way.
    #[arg(long = "set", global = true, value_name = "KEY=VALUE")]
    set: Vec<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write a commented vumlo.toml and .env.example. Never overwrites a file.
    Init {
        /// Where to write them.
        #[arg(long, default_value = ".", value_name = "DIR")]
        dir: PathBuf,
        /// Say what would be written, write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Check or show the configuration.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// List the modules and whether they are on.
    #[command(subcommand)]
    Modules(ModulesCommand),
    /// Run the subcommand of an optional module that is compiled in and enabled.
    Module {
        name: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// The database schema.
    #[command(subcommand)]
    Db(DbCommand),
    /// Accounts.
    #[command(subcommand)]
    Admin(AdminCommand),
    /// Check the installation without changing anything. Exits non-zero if a check fails.
    Doctor {
        #[arg(long)]
        json: bool,
    },
    /// Print the version of this binary and what it supports.
    Version,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Validate the configuration and list every problem.
    Check,
    /// Print the effective configuration (secrets redacted).
    Show {
        /// Only the keys that reach the browser: what client-config.json carries.
        #[arg(long)]
        public: bool,
    },
}

#[derive(Subcommand)]
enum ModulesCommand {
    List {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum DbCommand {
    /// What is applied and what is pending. Exits non-zero if the schema is newer than this binary.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Apply the pending migrations (core, then the modules compiled in). Refuses a database that
    /// is newer than this binary. Writes one audit row when it applied something.
    Migrate {
        /// Say what would be applied, apply nothing.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum AdminCommand {
    /// Create an administrator. The password is asked for, or read from standard input; it is
    /// never an argument.
    Create {
        email: String,
        /// First name.
        #[arg(long)]
        name: String,
        /// Last name.
        #[arg(long)]
        surname: String,
        /// Read the password from the first line of standard input instead of asking.
        #[arg(long)]
        password_stdin: bool,
        /// Check everything and say what would be created, create nothing.
        #[arg(long)]
        dry_run: bool,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// `Ok(false)` is a command that reported its own problems and only needs a failing exit code.
async fn run(cli: Cli) -> anyhow::Result<bool> {
    let options = Options {
        config: cli.config,
        set: cli.set,
    };
    match cli.command {
        Command::Init { dir, dry_run } => init::run(&dir, dry_run)?,
        Command::Config(ConfigCommand::Check) => return config_cmd::check(&options),
        Command::Config(ConfigCommand::Show { public }) => config_cmd::show(&options, public)?,
        Command::Modules(ModulesCommand::List { json }) => modules_cmd::list(&options, json)?,
        Command::Module { name, args } => modules_cmd::run_module(&options, &name, &args).await?,
        Command::Db(DbCommand::Status { json }) => return db_cmd::status(&options, json).await,
        Command::Db(DbCommand::Migrate { dry_run }) => db_cmd::migrate(&options, dry_run).await?,
        Command::Admin(AdminCommand::Create {
            email,
            name,
            surname,
            password_stdin,
            dry_run,
        }) => {
            admin::create(
                &options,
                admin::CreateAdmin {
                    email,
                    name,
                    surname,
                    password_stdin,
                    dry_run,
                },
            )
            .await?;
        }
        Command::Doctor { json } => return doctor::run(&options, json).await,
        Command::Version => version::run(),
    }
    Ok(true)
}
