//! A test-only module: the smallest thing that exercises every part of the module mechanism
//! (ADR-0054, ADR-0066) — a signed-in route with a permission, an event handler, a worker, its own
//! migrations and tables in `mod_dummy`, a subcommand, a doctor check, a private endpoint and a
//! secret. It is compiled in only by the `dummy` feature of `server` and `vumlo-cli`, which only
//! the test jobs of CI enable; no release build contains it.
//!
//! Its queries are checked at run time, not by `query!`: the module's schema is created by its own
//! migration, so the workspace builds and tests without it. A real module uses `query!`, and CI
//! prepares the offline data for every feature set.

use std::sync::LazyLock;
use std::time::Duration;

use access::Actor;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use common::{ApiError, Role};
use events::{HandlerError, HandlerFuture};
use module_api::{
    BoxFuture, Check, CheckStatus, CliExt, Manifest, Module, ModuleState, OutboundClient,
    Permission, Registrar, scoped_migrator,
};
use serde::{Deserialize, Serialize};
use sqlx::migrate::Migrator;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use vumlo_config::{SettingKind, SettingSpec};

pub const ID: &str = "dummy";
/// How often the worker writes a heartbeat row.
const HEARTBEAT_EVERY: Duration = Duration::from_millis(100);

static MANIFEST: Manifest = Manifest {
    id: ID,
    version: env!("CARGO_PKG_VERSION"),
    requires: &[],
    permissions: &[Permission {
        name: "items.write",
        roles: &[Role::Teacher, Role::Admin],
    }],
    settings: &[
        SettingSpec {
            key: "endpoint",
            kind: SettingKind::Endpoint,
            required: false,
        },
        SettingSpec {
            key: "api_key",
            kind: SettingKind::Secret {
                env: "VUMLO_DUMMY_API_KEY",
            },
            required: false,
        },
        SettingSpec {
            key: "label",
            kind: SettingKind::Text,
            required: false,
        },
    ],
    web: true,
};

static MIGRATOR: LazyLock<Migrator> =
    LazyLock::new(|| scoped_migrator(ID, sqlx::migrate!("./migrations")));

struct Dummy;

/// The module, for the one list in `server` and the one in `vumlo-cli`.
pub fn module() -> &'static dyn Module {
    &Dummy
}

impl Module for Dummy {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn migrations(&self) -> Option<&'static Migrator> {
        Some(&MIGRATOR)
    }

    fn register(&self, cx: &mut Registrar) {
        cx.routes(OpenApiRouter::new().routes(routes!(list_items, create_item)));
        cx.on("submission.submitted", see_event);
        cx.job(heartbeat);
    }

    fn cli(&self) -> Option<CliExt> {
        Some(CliExt { command, run })
    }

    fn doctor(&self, state: ModuleState) -> BoxFuture<Vec<Check>> {
        doctor(state)
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

/// The items of the dummy module. Any signed-in user.
#[utoipa::path(get, path = "/items", responses((status = 200, body = Vec<Item>)))]
async fn list_items(
    _actor: Actor,
    State(state): State<ModuleState>,
) -> Result<Json<Vec<Item>>, ApiError> {
    let rows: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id, label FROM mod_dummy.items ORDER BY created_at, id")
            .fetch_all(&state.db)
            .await
            .map_err(|e| ApiError::from_db("dummy", &e))?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, label)| Item { id, label })
            .collect(),
    ))
}

/// Adds an item. The permission `items.write`: teachers and admins.
#[utoipa::path(post, path = "/items", request_body = NewItem, responses((status = 201, body = Item)))]
async fn create_item(
    actor: Actor,
    State(state): State<ModuleState>,
    Json(new): Json<NewItem>,
) -> Result<(StatusCode, Json<Item>), ApiError> {
    state.authorize(&actor, "items.write")?;
    let label = new.label.trim().to_owned();
    if label.is_empty() || label.chars().count() > 100 {
        return Err(ApiError::invalid_field("label", "1 to 100 characters"));
    }
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO mod_dummy.items (id, label) VALUES ($1, $2)")
        .bind(id)
        .bind(&label)
        .execute(&state.db)
        .await
        .map_err(|e| ApiError::from_db("dummy", &e))?;
    Ok((StatusCode::CREATED, Json(Item { id, label })))
}

/// Remembers the id of a submitted work; a repeat changes nothing.
fn see_event(state: ModuleState, event: events::OutboxEvent) -> HandlerFuture {
    Box::pin(async move {
        sqlx::query(
            "INSERT INTO mod_dummy.seen_events (event_id) VALUES ($1) ON CONFLICT DO NOTHING",
        )
        .bind(event.id)
        .execute(&state.db)
        .await
        .map_err(|_| HandlerError("the dummy module could not store the event".to_owned()))?;
        Ok(())
    })
}

/// One task that writes a heartbeat row until the database goes away.
fn heartbeat(state: ModuleState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let beat = sqlx::query("INSERT INTO mod_dummy.heartbeat (id) VALUES ($1)")
                .bind(Uuid::now_v7())
                .execute(&state.db)
                .await;
            if beat.is_err() {
                return;
            }
            tokio::time::sleep(HEARTBEAT_EVERY).await;
        }
    })
}

fn command() -> clap::Command {
    clap::Command::new(ID)
        .about("The test-only module")
        .subcommand_required(true)
        .subcommand(clap::Command::new("count").about("Print how many items are stored"))
}

fn run(matches: clap::ArgMatches, state: ModuleState) -> BoxFuture<anyhow::Result<()>> {
    Box::pin(async move {
        match matches.subcommand() {
            Some(("count", _)) => {
                let count: i64 = sqlx::query_scalar("SELECT count(*) FROM mod_dummy.items")
                    .fetch_one(&state.db)
                    .await?;
                println!("{count}");
                Ok(())
            }
            _ => anyhow::bail!("expected a subcommand: count"),
        }
    })
}

fn doctor(ctx: ModuleState) -> BoxFuture<Vec<Check>> {
    Box::pin(async move {
        let mut checks = Vec::new();
        let items: Result<i64, _> = sqlx::query_scalar("SELECT count(*) FROM mod_dummy.items")
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
