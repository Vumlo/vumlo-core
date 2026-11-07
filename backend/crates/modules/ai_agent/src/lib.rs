//! The Buddy (ADR-0070, ADR-0071): a chat agent for students, teachers and admins, built on
//! `module-api`. It asks an OpenAI-compatible model, looks things up through the service
//! functions of `learning` as the user, and keeps one rolling chat per user in `mod_ai_agent`.
//! The module id is `ai_agent`; "assistant" is a course role and is not used for it.

mod config;
mod llm;
mod model;
mod persona;
mod repo;
mod routes;
mod service;
mod tools;

use std::sync::LazyLock;
use std::time::Duration;

use common::Role;
use module_api::{
    BoxFuture, Check, CheckStatus, Manifest, Module, ModuleState, OutboundClient, Permission,
    Registrar, scoped_migrator,
};
use sqlx::migrate::Migrator;
use vumlo_config::{SettingKind, SettingSpec};

use crate::config::Config;
use crate::persona::Persona;

pub const ID: &str = "ai_agent";
/// How often old chats are deleted.
const PURGE_EVERY: Duration = Duration::from_secs(60 * 60);

static MANIFEST: Manifest = Manifest {
    id: ID,
    version: env!("CARGO_PKG_VERSION"),
    requires: &[],
    // One permission per kind of user; `Persona::of` finds the kind by asking `access`.
    permissions: &[
        Permission {
            name: "chat.student",
            roles: &[Role::Student],
        },
        Permission {
            name: "chat.teacher",
            roles: &[Role::Teacher],
        },
        Permission {
            name: "chat.admin",
            roles: &[Role::Admin],
        },
    ],
    settings: &[
        // The base URL of an OpenAI-compatible API, such as https://api.openai.com/v1.
        SettingSpec {
            key: "endpoint",
            kind: SettingKind::Endpoint,
            required: true,
        },
        SettingSpec {
            key: "model",
            kind: SettingKind::Text,
            required: true,
        },
        SettingSpec {
            key: "api_key",
            kind: SettingKind::Secret {
                env: "VUMLO_AI_AGENT_API_KEY",
            },
            required: false,
        },
        // Who gets the Buddy: a comma-separated list of student, teacher, admin. Default: teacher,admin.
        SettingSpec {
            key: "roles",
            kind: SettingKind::Text,
            required: false,
        },
        // Messages per user per day. Default 100; 0 is no limit.
        SettingSpec {
            key: "daily_cap",
            kind: SettingKind::Int,
            required: false,
        },
        // Days a chat is kept. Default 30.
        SettingSpec {
            key: "retention_days",
            kind: SettingKind::Int,
            required: false,
        },
        // Shown before the first message: where the messages go.
        SettingSpec {
            key: "notice",
            kind: SettingKind::Text,
            required: false,
        },
    ],
    web: true,
};

static MIGRATOR: LazyLock<Migrator> =
    LazyLock::new(|| scoped_migrator(ID, sqlx::migrate!("./migrations")));

struct AiAgent;

/// The module, for the one list in `server` and the one in `vumlo-cli`.
pub fn module() -> &'static dyn Module {
    &AiAgent
}

impl Module for AiAgent {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn migrations(&self) -> Option<&'static Migrator> {
        Some(&MIGRATOR)
    }

    fn register(&self, cx: &mut Registrar) {
        cx.routes(routes::router());
        cx.job(retention);
    }

    fn doctor(&self, state: ModuleState) -> BoxFuture<Vec<Check>> {
        doctor(state)
    }
}

/// Deletes chats older than `retention_days`, once an hour.
fn retention(state: ModuleState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let days = Config::new(&state.settings).retention_days();
            match state.db.acquire().await {
                Ok(mut conn) => {
                    if let Err(error) = service::purge(&mut conn, days).await {
                        tracing::warn!(%error, "ai_agent: could not delete old chats");
                    }
                }
                Err(error) => tracing::warn!(%error, "ai_agent: no connection to delete old chats"),
            }
            tokio::time::sleep(PURGE_EVERY).await;
        }
    })
}

fn doctor(state: ModuleState) -> BoxFuture<Vec<Check>> {
    Box::pin(async move {
        let mut checks = Vec::new();
        let chats: Result<i64, _> =
            sqlx::query_scalar("SELECT count(*) FROM mod_ai_agent.messages")
                .fetch_one(&state.db)
                .await;
        checks.push(match chats {
            Ok(n) if n > 0 && !state.settings.enabled => Check::new(
                "ai_agent data",
                CheckStatus::Warn,
                format!("the module is off, so old chats are not deleted: {n} chat message(s) stay stored until it is switched on again"),
            ),
            Ok(n) => Check::new("ai_agent data", CheckStatus::Ok, format!("{n} chat message(s)")),
            Err(_) => Check::new(
                "ai_agent data",
                CheckStatus::Fail,
                "the ai_agent schema is missing; run `vumlo-cli db migrate`",
            ),
        });
        if !state.settings.enabled {
            return checks;
        }
        let config = Config::new(&state.settings);

        let tokens = config.role_tokens();
        let unknown: Vec<&str> = tokens
            .iter()
            .map(String::as_str)
            .filter(|t| !Persona::ALL.iter().any(|p| p.token() == *t))
            .collect();
        checks.push(if unknown.is_empty() {
            Check::new("ai_agent roles", CheckStatus::Ok, tokens.join(", "))
        } else {
            Check::new(
                "ai_agent roles",
                CheckStatus::Warn,
                format!(
                    "unknown role(s) {}: use student, teacher or admin",
                    unknown.join(", ")
                ),
            )
        });

        if let Some(endpoint) = config.endpoint() {
            // Reachability only: the key and any user data stay home.
            let reached = match OutboundClient::new(
                Duration::from_secs(3),
                Duration::from_secs(5),
                64 * 1024,
                0,
            ) {
                Ok(client) => client.probe(endpoint).await.map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            checks.push(match reached {
                Ok(status) => Check::new(
                    "ai_agent model endpoint",
                    CheckStatus::Ok,
                    format!("answers (HTTP {status})"),
                ),
                Err(why) => Check::new("ai_agent model endpoint", CheckStatus::Warn, why),
            });
        }
        checks
    })
}
