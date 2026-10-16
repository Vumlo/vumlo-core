use std::collections::HashSet;
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use std::net::SocketAddr;

use axum::http::StatusCode;
use axum::middleware::map_response;
use axum::routing::get;
use axum::{Json, Router};
use sqlx::PgPool;
use sqlx::migrate::{Migrate, MigrateError, Migrator};
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use utoipa_scalar::{Scalar, Servable};

use crate::content;
use crate::error::{ApiError, problem_details};
use crate::health;
use crate::identity::{self, SetupError};
use crate::settings::Settings;
use crate::teaching;

pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// Shorter than the request timeout, so an unreachable database surfaces as 503, not 408.
const DB_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Room for a 1 MiB document (ADR-0025) and the rest of the request around it.
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Settings>,
}

#[derive(OpenApi)]
#[openapi(info(title = "Learnly API"), modifiers(&SecurityAddon))]
struct ApiDoc;

/// The two ways to authenticate (ADR-0003): the web cookie and the bearer token.
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "cookie",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new(identity::COOKIE_NAME))),
        );
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some(
                        "A session token from `POST /auth/login` with `client: app`.",
                    ))
                    .build(),
            ),
        );
    }
}

pub async fn connect(settings: &Settings) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(settings.database.max_connections)
        .acquire_timeout(DB_ACQUIRE_TIMEOUT)
        .connect(&settings.database.url)
        .await
}

#[derive(Debug)]
pub enum MigrationCheckError {
    Db(sqlx::Error),
    Migrate(MigrateError),
    Pending(usize),
}

impl fmt::Display for MigrationCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Db(e) => write!(f, "cannot read the migration state: {e}"),
            Self::Migrate(e) => write!(f, "cannot read the migration state: {e}"),
            Self::Pending(n) => {
                write!(
                    f,
                    "{n} database migration(s) pending; run `learnly-server migrate` first"
                )
            }
        }
    }
}

impl std::error::Error for MigrationCheckError {}

impl From<sqlx::Error> for MigrationCheckError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

impl From<MigrateError> for MigrationCheckError {
    fn from(e: MigrateError) -> Self {
        Self::Migrate(e)
    }
}

/// Fails if any embedded migration has not been applied. Never changes the database.
pub async fn ensure_migrated(pool: &PgPool) -> Result<(), MigrationCheckError> {
    let mut conn = pool.acquire().await?;
    let table_exists =
        sqlx::query!(r#"SELECT to_regclass('_sqlx_migrations') IS NOT NULL AS "exists!""#)
            .fetch_one(&mut *conn)
            .await?
            .exists;
    let applied: HashSet<i64> = if table_exists {
        conn.list_applied_migrations(&MIGRATOR.table_name)
            .await?
            .into_iter()
            .map(|m| m.version)
            .collect()
    } else {
        HashSet::new()
    };
    let pending = MIGRATOR
        .iter()
        .filter(|m| !m.migration_type.is_down_migration() && !applied.contains(&m.version))
        .count();
    if pending == 0 {
        Ok(())
    } else {
        Err(MigrationCheckError::Pending(pending))
    }
}

pub fn router(state: AppState) -> Result<Router, SetupError> {
    let identity = identity::router(&state.config)?;
    let (router, openapi) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .nest(
            "/api/v1",
            OpenApiRouter::new()
                .routes(routes!(health::health))
                .merge(identity)
                .merge(teaching::router())
                .merge(content::router()),
        )
        .split_for_parts();
    let spec = openapi.clone();
    Ok(router
        .route(
            "/api/v1/openapi.json",
            get(move || async move { Json(spec) }),
        )
        .merge(Scalar::with_url("/api/v1/docs", openapi))
        .fallback(|| async { ApiError::NotFound })
        .with_state(state)
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(map_response(problem_details)))
}

pub async fn serve(
    listener: TcpListener,
    router: Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
}

/// Registers SIGINT and SIGTERM handlers now; the returned future completes on the first signal.
pub fn shutdown_signal() -> std::io::Result<impl Future<Output = ()>> {
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;
    Ok(async move {
        tokio::select! {
            _ = sigint.recv() => {},
            _ = sigterm.recv() => {},
        }
    })
}
