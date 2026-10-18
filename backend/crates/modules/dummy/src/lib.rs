//! A test-only module: the smallest thing that exercises every part of the module mechanism
//! (ADR-0054) — a route, a worker, its own migrations and table, a subcommand, a doctor check,
//! a private endpoint and a secret. It is compiled in only by the `dummy` feature of `server` and
//! `vumlo-cli`, which only the test jobs of CI enable; no release build contains it.
//!
//! Its queries are checked at run time, not by `query!`: the module's schema is created by its own
//! migration, so the workspace builds and tests without it. A real module uses `query!`, and CI
//! prepares the offline data for every feature set.

use std::sync::LazyLock;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use module_api::{BoxFuture, Check, CheckStatus, CliExt, ModuleContext, ModuleDef, OutboundClient};
use serde::{Deserialize, Serialize};
use sqlx::migrate::Migrator;
use tokio::task::JoinHandle;
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

pub const NAME: &str = "dummy";
/// How often the worker writes a heartbeat row.
const HEARTBEAT_EVERY: Duration = Duration::from_millis(100);

static MIGRATOR: LazyLock<Migrator> = LazyLock::new(|| {
    let mut migrator = sqlx::migrate!("./migrations");
    migrator.dangerous_set_table_name("_sqlx_migrations_dummy");
    migrator
});

pub fn def() -> ModuleDef {
    ModuleDef {
        name: NAME,
        migrator: Some(&MIGRATOR),
        router,
        openapi: Doc::openapi,
        workers,
        cli: Some(CliExt { command, run }),
        doctor,
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Item {
    pub id: Uuid,
    pub label: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct NewItem {
    pub label: String,
}

#[derive(OpenApi)]
#[openapi(paths(list_items, create_item), components(schemas(Item, NewItem)))]
struct Doc;

fn router(ctx: ModuleContext) -> Router {
    Router::new()
        .route("/items", get(list_items).post(create_item))
        .with_state(ctx)
}

/// The items of the dummy module.
#[utoipa::path(get, path = "/items", responses((status = 200, body = Vec<Item>)))]
async fn list_items(State(ctx): State<ModuleContext>) -> Result<Json<Vec<Item>>, StatusCode> {
    let rows: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id, label FROM dummy.items ORDER BY created_at, id")
            .fetch_all(&ctx.db)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, label)| Item { id, label })
            .collect(),
    ))
}

/// Adds an item.
#[utoipa::path(post, path = "/items", request_body = NewItem, responses((status = 201, body = Item)))]
async fn create_item(
    State(ctx): State<ModuleContext>,
    Json(new): Json<NewItem>,
) -> Result<(StatusCode, Json<Item>), StatusCode> {
    let label = new.label.trim().to_owned();
    if label.is_empty() || label.chars().count() > 100 {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO dummy.items (id, label) VALUES ($1, $2)")
        .bind(id)
        .bind(&label)
        .execute(&ctx.db)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok((StatusCode::CREATED, Json(Item { id, label })))
}

/// One task that writes a heartbeat row until the database goes away.
fn workers(ctx: ModuleContext) -> Vec<JoinHandle<()>> {
    vec![tokio::spawn(async move {
        loop {
            let beat = sqlx::query("INSERT INTO dummy.heartbeat (id) VALUES ($1)")
                .bind(Uuid::now_v7())
                .execute(&ctx.db)
                .await;
            if beat.is_err() {
                return;
            }
            tokio::time::sleep(HEARTBEAT_EVERY).await;
        }
    })]
}

fn command() -> clap::Command {
    clap::Command::new(NAME)
        .about("The test-only module")
        .subcommand_required(true)
        .subcommand(clap::Command::new("count").about("Print how many items are stored"))
}

fn run(matches: clap::ArgMatches, ctx: ModuleContext) -> BoxFuture<anyhow::Result<()>> {
    Box::pin(async move {
        match matches.subcommand() {
            Some(("count", _)) => {
                let count: i64 = sqlx::query_scalar("SELECT count(*) FROM dummy.items")
                    .fetch_one(&ctx.db)
                    .await?;
                println!("{count}");
                Ok(())
            }
            _ => anyhow::bail!("expected a subcommand: count"),
        }
    })
}

fn doctor(ctx: ModuleContext) -> BoxFuture<Vec<Check>> {
    Box::pin(async move {
        let mut checks = Vec::new();
        let items: Result<i64, _> = sqlx::query_scalar("SELECT count(*) FROM dummy.items")
            .fetch_one(&ctx.db)
            .await;
        match items {
            Ok(n) if n > 0 && !ctx.settings.enabled => checks.push(Check::new(
                "dummy data",
                CheckStatus::Warn,
                format!("the module is off, but {n} item(s) stay stored (data is never deleted)"),
            )),
            Ok(n) => checks.push(Check::new(
                "dummy data",
                CheckStatus::Ok,
                format!("{n} item(s)"),
            )),
            Err(_) => checks.push(Check::new(
                "dummy data",
                CheckStatus::Fail,
                "the dummy schema is missing; run `vumlo-cli db migrate`",
            )),
        }
        if !ctx.settings.enabled {
            return checks;
        }
        let endpoint = ctx
            .settings
            .settings
            .get("endpoint")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        let Some(endpoint) = endpoint else {
            return checks;
        };
        if !ctx.settings.secrets.contains_key("api_key") {
            checks.push(Check::new(
                "dummy endpoint",
                CheckStatus::Fail,
                "an endpoint is set but VUMLO_DUMMY_API_KEY is not",
            ));
            return checks;
        }
        // Reachability only: the key and any user data stay home.
        let client =
            OutboundClient::new(Duration::from_secs(3), Duration::from_secs(5), 64 * 1024, 0);
        let reached = match client {
            Ok(client) => client.probe(&endpoint).await.map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        checks.push(match reached {
            Ok(status) => Check::new(
                "dummy endpoint",
                CheckStatus::Ok,
                format!("answers (HTTP {status})"),
            ),
            Err(why) => Check::new("dummy endpoint", CheckStatus::Warn, why),
        });
        checks
    })
}
