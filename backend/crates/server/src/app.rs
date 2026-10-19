use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use std::net::SocketAddr;

use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, map_response};
use axum::routing::get;
use axum::{Json, Router};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use utoipa_scalar::{Scalar, Servable};

use crate::assessment;
use crate::content::{self, EditorConfigError, EffectiveProfiles};
use crate::error::{ApiError, problem_details};
use crate::events::Handlers;
use crate::gradebook;
use crate::health;
use crate::identity::{self, SetupError};
use crate::mail::{MailError, Mailer};
use crate::org;
use crate::teaching;
use vumlo_config::Settings;

// Shared with `vumlo-cli`: the migrations live in the `db` crate.
pub use db::{MIGRATOR, MigrationCheckError, ensure_migrated};

/// Shorter than the request timeout, so an unreachable database surfaces as 503, not 408.
const DB_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Room for a 1 MiB document (ADR-0025) and the rest of the request around it.
pub(crate) const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Settings>,
    /// The editor profiles, resolved once from `config.editor` (ADR-0025, ADR-0027).
    pub editor: Arc<EffectiveProfiles>,
    /// The bound on password hashes and checks that run at once (ADR-0038).
    pub hashing: Arc<identity::HashGate>,
    /// Outgoing mail (ADR-0045); `None` without `[mail]`, and the features that must send mail are off.
    pub mailer: Option<Arc<Mailer>>,
    /// Sign-in with Google (ADR-0047); `None` without `[auth.google]`, and its endpoints are `404`.
    pub google: Option<Arc<identity::Google>>,
}

/// The configuration cannot be applied. The message names the key and the value.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error(transparent)]
    Editor(#[from] EditorConfigError),
    #[error(transparent)]
    Mail(#[from] MailError),
    #[error(transparent)]
    Google(#[from] identity::GoogleSetupError),
}

impl AppState {
    /// The composition root of the shared state. It resolves the editor configuration, so a
    /// configuration that does not fit the profiles in code fails here, at startup, with an error
    /// that names the key and the value, and never at the first request.
    pub fn new(db: PgPool, config: Settings) -> Result<Self, StateError> {
        let editor = EffectiveProfiles::resolve(&config.editor)?;
        let hashing = identity::HashGate::new(config.auth.max_concurrent_hashes);
        let mailer = config
            .mail
            .as_ref()
            .map(|mail| Mailer::new(mail, &config.server.public_origin).map(Arc::new))
            .transpose()?;
        let google = config
            .auth
            .google
            .as_ref()
            .map(|google| identity::Google::new(google).map(Arc::new))
            .transpose()?;
        Ok(Self {
            db,
            config: Arc::new(config),
            editor: Arc::new(editor),
            hashing: Arc::new(hashing),
            mailer,
            google,
        })
    }

    /// The outbox handlers of every module (ADR-0045), for `events::run_worker` and for tests that
    /// deliver by hand with `events::process_once`.
    pub fn event_handlers(&self) -> Handlers {
        let mut handlers = Handlers::default();
        if let Some(mailer) = &self.mailer {
            handlers = handlers.merge(identity::event_handlers(
                self.db.clone(),
                identity::MailContext {
                    mailer: mailer.clone(),
                    language: self.config.institution.language,
                    institution_name: self.config.institution.name.clone(),
                },
            ));
        }
        handlers
    }
}

#[derive(OpenApi)]
#[openapi(info(title = "Vumlo API"), modifiers(&SecurityAddon))]
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
        .connect(settings.database.url.expose())
        .await
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
                .merge(content::router())
                .merge(gradebook::router())
                .merge(assessment::router())
                .merge(org::router())
                .merge(crate::events::router()),
        )
        .split_for_parts();
    let (router, openapi) = mount_modules(&state, router, openapi);
    let spec = openapi.clone();
    let config = state.config.clone();
    let idempotency = state.clone();
    let router = router
        .route(
            "/api/v1/openapi.json",
            get(move || async move { Json(spec) }),
        )
        .merge(Scalar::with_url("/api/v1/docs", openapi))
        .fallback(|| async { ApiError::NotFound })
        .with_state(state)
        // Innermost, so every handler sits behind it and every outer layer still sees its answers (ADR-0058).
        .layer(middleware::from_fn_with_state(
            idempotency,
            crate::idempotency::layer,
        ))
        .layer(middleware::from_fn(crate::conditional::layer))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::http::Request<_>| {
                    // The path, never the query: a query can carry an email address (AGENTS §11).
                    tracing::info_span!(
                        "request",
                        method = %request.method(),
                        path = %request.uri().path(),
                        version = ?request.version(),
                    )
                })
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(map_response(problem_details));
    Ok(cors(router, &config.server.allowed_origins))
}

/// Mounts every module that is compiled in and enabled under `/api/v1/<name>`, and adds its paths
/// to the OpenAPI document. A module that is off gets nothing: its paths answer `404` and are not
/// in the document (ADR-0054).
fn mount_modules(
    state: &AppState,
    mut router: Router<AppState>,
    mut openapi: utoipa::openapi::OpenApi,
) -> (Router<AppState>, utoipa::openapi::OpenApi) {
    for module in crate::modules::compiled() {
        if !state.config.module_state.is_enabled(module.name) {
            continue;
        }
        let prefix = format!("/api/v1/{}", module.name);
        router = router.nest_service(
            &prefix,
            (module.router)(crate::modules::context(state, module.name)),
        );
        let mut doc = (module.openapi)();
        doc.paths.paths = std::mem::take(&mut doc.paths.paths)
            .into_iter()
            .map(|(path, item)| (format!("{prefix}{path}"), item))
            .collect();
        openapi.merge(doc);
    }
    (router, openapi)
}

/// How long a browser may reuse a preflight answer.
const PREFLIGHT_MAX_AGE: Duration = Duration::from_secs(600);

/// Lets the browser frontends of `allowed_origins` call the API (ADR-0041). None listed: the
/// router is returned as it is and no CORS header is ever sent. The layer goes on last so it is
/// outermost: a preflight is answered before the rate limit, the credential and the CSRF check,
/// and the headers also reach the error bodies that `problem_details` rebuilds.
fn cors(router: Router, allowed_origins: &[String]) -> Router {
    if allowed_origins.is_empty() {
        return router;
    }
    let allowed: Vec<HeaderValue> = allowed_origins
        .iter()
        .filter_map(|origin| HeaderValue::from_str(origin).ok())
        .collect();
    router.layer(
        CorsLayer::new()
            .allow_origin(AllowOrigin::predicate(move |origin, _| {
                allowed.contains(origin)
            }))
            .allow_credentials(true)
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
            ])
            .allow_headers([
                header::AUTHORIZATION,
                header::CONTENT_TYPE,
                header::IF_NONE_MATCH,
                crate::idempotency::KEY_HEADER,
            ])
            .expose_headers([
                header::RETRY_AFTER,
                header::ETAG,
                crate::idempotency::REPLAYED_HEADER,
            ])
            .max_age(PREFLIGHT_MAX_AGE),
    )
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
