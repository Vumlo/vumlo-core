use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use server::app::{self, AppState};
use server::settings::{LogFormat, Settings};

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
            app::ensure_migrated(&pool).await?;
            let listener = tokio::net::TcpListener::bind(settings.server.bind_addr)
                .await
                .with_context(|| format!("cannot bind {}", settings.server.bind_addr))?;
            let shutdown = app::shutdown_signal().context("cannot install signal handlers")?;
            tracing::info!(addr = %settings.server.bind_addr, "listening");
            let state = AppState {
                db: pool,
                config: std::sync::Arc::new(settings),
            };
            app::serve(listener, app::router(state), shutdown)
                .await
                .context("server error")?;
            tracing::info!("shut down");
        }
        Command::Migrate => {
            app::MIGRATOR.run(&pool).await.context("migration failed")?;
            tracing::info!("migrations applied");
        }
    }
    Ok(())
}

fn init_tracing(format: LogFormat) {
    let builder = tracing_subscriber::fmt().with_max_level(tracing::Level::INFO);
    match format {
        LogFormat::Pretty => builder.init(),
        LogFormat::Json => builder.json().init(),
    }
}
