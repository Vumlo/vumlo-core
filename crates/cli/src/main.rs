use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, bail};
use api_types::{HealthResponse, ProblemDetails};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "learnly", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Call GET /api/v1/health; exit non-zero if the server is not healthy.
    Health {
        /// Base URL of the server, e.g. http://localhost:8080
        #[arg(long)]
        url: String,
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
    match cli.command {
        Command::Health { url } => {
            let endpoint = format!("{}/api/v1/health", url.trim_end_matches('/'));
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()?;
            let response = client
                .get(&endpoint)
                .send()
                .await
                .with_context(|| format!("cannot reach {endpoint}"))?;
            let status = response.status().as_u16();
            let body = response
                .bytes()
                .await
                .context("cannot read the response body")?;
            let health = check(status, &body)?;
            println!(
                "OK (status: {}, database: {})",
                health.status, health.database
            );
            Ok(())
        }
    }
}

/// Turns a health response into `Ok` only when the server reports itself fully healthy.
fn check(status: u16, body: &[u8]) -> anyhow::Result<HealthResponse> {
    if status != 200 {
        let Ok(p) = serde_json::from_slice::<ProblemDetails>(body) else {
            bail!("unexpected response status {status}");
        };
        bail!(
            "unhealthy: {} {}: {}",
            p.status,
            p.title,
            p.detail.unwrap_or_default()
        );
    }
    let health: HealthResponse =
        serde_json::from_slice(body).context("the server returned an invalid health response")?;
    if health.status != "ok" || health.database != "ok" {
        bail!(
            "unhealthy: status={}, database={}",
            health.status,
            health.database
        );
    }
    Ok(health)
}

#[cfg(test)]
mod tests {
    use super::check;

    #[test]
    fn healthy_response_is_accepted() {
        let health = check(200, br#"{"status":"ok","database":"ok"}"#).unwrap();
        assert_eq!(health.database, "ok");
    }

    #[test]
    fn degraded_body_is_rejected() {
        let e = check(200, br#"{"status":"ok","database":"down"}"#).unwrap_err();
        assert!(e.to_string().contains("database=down"), "{e}");
    }

    #[test]
    fn problem_details_are_reported() {
        let body = br#"{"type":"about:blank","title":"Service Unavailable","status":503,"detail":"database unavailable"}"#;
        let e = check(503, body).unwrap_err();
        assert!(e.to_string().contains("database unavailable"), "{e}");
    }

    #[test]
    fn unexpected_status_without_problem_body_is_rejected() {
        let e = check(502, b"<html>bad gateway</html>").unwrap_err();
        assert!(e.to_string().contains("502"), "{e}");
    }

    #[test]
    fn garbage_body_is_rejected() {
        assert!(check(200, b"nope").is_err());
    }
}
