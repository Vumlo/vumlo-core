use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use server::app::{self, AppState};
use server::identity::{self, Email, NewUser, PersonName, Role, TemporaryPassword};
use server::org;
use server::settings::{LogFormat, Settings};
use sqlx::PgPool;

#[derive(Parser)]
#[command(name = "learnly-server", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the HTTP server. Refuses to start while migrations are pending.
    Serve,
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
    let settings = Settings::load().context("invalid configuration")?;
    init_tracing(settings.log.format);
    let pool = app::connect(&settings)
        .await
        .context("cannot connect to the database")?;
    match cli.command {
        Command::Serve => {
            let state = AppState::new(pool, settings).context("invalid configuration")?;
            prepare_database(&state.db, &state.config).await?;
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
            app::MIGRATOR.run(&pool).await.context("migration failed")?;
            record_mode(&pool, &settings).await?;
            tracing::info!("migrations applied");
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
    app::ensure_migrated(pool).await?;
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
