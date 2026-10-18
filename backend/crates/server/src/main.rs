use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use server::app::{self, AppState};
use server::events;
use server::identity::{self, Email, NewUser, PersonName, Role, TemporaryPassword};
use server::modules;
use server::org;
use sqlx::PgPool;
use vumlo_config::{LogFormat, Options, Settings};

#[derive(Parser)]
#[command(name = "vumlo-server", version)]
struct Cli {
    /// The configuration file. Default: ./vumlo.toml, then ../vumlo.toml.
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<std::path::PathBuf>,
    /// Override one key, e.g. `--set server.bind_addr=0.0.0.0:8080`. Secrets cannot be set this way.
    #[arg(long = "set", global = true, value_name = "KEY=VALUE")]
    set: Vec<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the HTTP server and the background work. Refuses to start while migrations are pending.
    Serve {
        /// Serve requests only. Run `vumlo-server worker` as a separate process for the background
        /// work (the outbox, the mail and the workers of the modules that are on).
        #[arg(long)]
        no_worker: bool,
    },
    /// Run the background work without serving requests, until SIGINT or SIGTERM.
    Worker,
    /// Apply embedded database migrations and exit.
    Migrate,
    /// Create an admin with a temporary password, printed once.
    CreateAdmin {
        email: String,
        /// First name.
        #[arg(long)]
        name: String,
        /// Last name.
        #[arg(long)]
        surname: String,
    },
    /// Create a user with a temporary password, printed once.
    CreateUser {
        email: String,
        /// First name.
        #[arg(long)]
        name: String,
        /// Last name.
        #[arg(long)]
        surname: String,
        /// admin, teacher or student.
        #[arg(long)]
        role: Role,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let options = Options {
        config: cli.config,
        set: cli.set,
    };
    let settings = vumlo_config::load_process(&options, &modules::compiled_names())
        .context("invalid configuration")?
        .settings;
    init_tracing(settings.log.format);
    let pool = app::connect(&settings)
        .await
        .context("cannot connect to the database")?;
    match cli.command {
        Command::Worker => {
            let state = AppState::new(pool, settings).context("invalid configuration")?;
            prepare_database(&state.db, &state.config).await?;
            let started = modules::spawn_workers(&state);
            let shutdown = app::shutdown_signal().context("cannot install signal handlers")?;
            tracing::info!(modules = ?started, "worker running");
            tokio::select! {
                () = events::run_worker(state.db.clone(), state.event_handlers()) => {}
                () = shutdown => {}
            }
            tracing::info!("shut down");
        }
        Command::Serve { no_worker } => {
            let state = AppState::new(pool, settings).context("invalid configuration")?;
            prepare_database(&state.db, &state.config).await?;
            if !no_worker {
                tokio::spawn(events::run_worker(state.db.clone(), state.event_handlers()));
                let started = modules::spawn_workers(&state);
                tracing::info!(modules = ?started, "background work started");
            }
            let bind_addr = state.config.server.bind_addr;
            let listener = tokio::net::TcpListener::bind(bind_addr)
                .await
                .with_context(|| format!("cannot bind {bind_addr}"))?;
            let shutdown = app::shutdown_signal().context("cannot install signal handlers")?;
            tracing::info!(addr = %bind_addr, "listening");
            app::serve(
                listener,
                app::router(state).context("cannot build the router")?,
                shutdown,
            )
            .await
            .context("server error")?;
            tracing::info!("shut down");
        }
        Command::Migrate => {
            let mut applied = db::migrate(&pool, "core", &db::MIGRATOR)
                .await
                .context("migration failed")?
                .len();
            for (name, migrator) in modules::migrators() {
                applied += db::migrate(&pool, name, migrator)
                    .await
                    .with_context(|| format!("migration of the module {name} failed"))?
                    .len();
            }
            record_mode(&pool, &settings).await?;
            tracing::info!(applied, "migrations applied");
        }
        Command::CreateAdmin {
            email,
            name,
            surname,
        } => {
            create_account(&pool, &settings, &email, &name, &surname, Role::Admin).await?;
        }
        Command::CreateUser {
            email,
            name,
            surname,
            role,
        } => {
            create_account(&pool, &settings, &email, &name, &surname, role).await?;
        }
    }
    Ok(())
}

/// Refuses a database with pending migrations, then records or checks the institution mode
/// (ADR-0031). Every command that uses the database goes through this before it does anything.
async fn prepare_database(pool: &PgPool, settings: &Settings) -> anyhow::Result<()> {
    db::ensure_migrated_with(pool, &modules::migrators()).await?;
    record_mode(pool, settings).await
}

async fn record_mode(pool: &PgPool, settings: &Settings) -> anyhow::Result<()> {
    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    org::record_mode(&mut tx, settings.institution.mode).await?;
    tx.commit().await.context("cannot commit")?;
    Ok(())
}

/// Creates a user with a temporary password (ADR-0017) and prints the password. This is the
/// only time it is visible; the user must change it at first login.
async fn create_account(
    pool: &PgPool,
    settings: &Settings,
    email: &str,
    name: &str,
    surname: &str,
    role: Role,
) -> anyhow::Result<()> {
    prepare_database(pool, settings).await?;
    let new = NewUser {
        email: Email::parse(email).context("invalid email")?,
        name: PersonName::name(name).context("invalid name")?,
        surname: PersonName::surname(surname).context("invalid surname")?,
        role,
    };
    let temporary = TemporaryPassword::generate();
    // Hash before the transaction opens (ADR-0016).
    let hash = identity::hash_password(temporary.expose())
        .await
        .context("cannot hash the password")?;
    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    let user = identity::create_user_from_cli(&mut tx, new, hash)
        .await
        .context("cannot create the user")?;
    tx.commit().await.context("cannot commit")?;
    tracing::info!(user_id = %user.id, role = role.as_str(), "user created");
    println!("Created {} {}", role.as_str(), user.id);
    println!("Temporary password: {}", temporary.expose());
    println!("It is shown only once and must be changed at first login.");
    Ok(())
}

fn init_tracing(format: LogFormat) {
    let builder = tracing_subscriber::fmt().with_max_level(tracing::Level::INFO);
    match format {
        LogFormat::Pretty => builder.init(),
        LogFormat::Json => builder.json().init(),
    }
}
