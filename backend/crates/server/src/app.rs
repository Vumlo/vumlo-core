use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use std::net::SocketAddr;

use axum::extract::FromRef;
use axum::http::request::Parts;
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

use crate::health;
use access::{self, ActorSource, PluginSource};
use assessment::AssessmentState;
use auth::{self, AuthState, SetupError};
use common::{ApiError, problem_details};
use content::{ContentState, EditorConfigError, EffectiveProfiles};
use events::Handlers;
use gradebook::GradebookState;
use learning::{self, LearningState};
use mail::{MailError, MailState, Mailer};
use org::{self, OrgState};
use plugins::PluginsState;
use teaching::TeachingState;
use vumlo_config::Settings;

/// Shorter than the request timeout, so an unreachable database surfaces as 503, not 408.
const DB_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// An upload or a download of a file up to `files.max_size` over a slow link.
const FILE_REQUEST_TIMEOUT: Duration = Duration::from_secs(600);
/// A call through the core to a plugin's server: its own limit is 30 s, so this is the margin.
const PROXY_REQUEST_TIMEOUT: Duration = Duration::from_secs(45);
/// Room for a 1 MiB document (ADR-0025) and the rest of the request around it.
pub(crate) const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// The application's state: what each layer needs from it, and what the composition root keeps
/// for itself. Each layer's router asks for its own part (`FromRef`).
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Settings>,
    pub auth: AuthState,
    pub mail: MailState,
    /// The transport and what the mail worker needs besides it; `None` without `[mail]`.
    pub mailer: Option<Arc<Mailer>>,
    pub mail_context: Option<mail::Context>,
    pub org: OrgState,
    pub teaching: TeachingState,
    pub content: ContentState,
    pub assessment: AssessmentState,
    pub gradebook: GradebookState,
    pub learning: LearningState,
    pub plugins: PluginsState,
    /// The optional modules compiled in, each after the ones it requires.
    pub modules: Arc<Vec<&'static dyn module_api::Module>>,
}

impl FromRef<AppState> for MailState {
    fn from_ref(state: &AppState) -> Self {
        state.mail.clone()
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(state: &AppState) -> Self {
        state.auth.clone()
    }
}

impl FromRef<AppState> for OrgState {
    fn from_ref(state: &AppState) -> Self {
        state.org.clone()
    }
}

impl FromRef<AppState> for TeachingState {
    fn from_ref(state: &AppState) -> Self {
        state.teaching.clone()
    }
}

impl FromRef<AppState> for ContentState {
    fn from_ref(state: &AppState) -> Self {
        state.content.clone()
    }
}

impl FromRef<AppState> for AssessmentState {
    fn from_ref(state: &AppState) -> Self {
        state.assessment.clone()
    }
}

impl FromRef<AppState> for GradebookState {
    fn from_ref(state: &AppState) -> Self {
        state.gradebook.clone()
    }
}

impl FromRef<AppState> for LearningState {
    fn from_ref(state: &AppState) -> Self {
        state.learning.clone()
    }
}

impl FromRef<AppState> for PluginsState {
    fn from_ref(state: &AppState) -> Self {
        state.plugins.clone()
    }
}

impl PluginSource for AppState {
    async fn authenticate_plugin(
        &self,
        parts: &Parts,
    ) -> Result<access::AuthenticatedPlugin, ApiError> {
        self.plugins.authenticate_plugin(parts).await
    }
}

impl ActorSource for AppState {
    async fn authenticate(&self, parts: &Parts) -> Result<access::Authenticated, ApiError> {
        let session = auth::authenticate_request(parts, &self.auth).await?;
        Ok(access::Authenticated {
            user_id: session.user_id,
            role: session.role,
            session_id: session.session_id,
            audience: session.audience,
            must_change_password: session.must_change_password,
        })
    }
}

/// The configuration cannot be applied. The message names the key and the value.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error(transparent)]
    Editor(#[from] EditorConfigError),
    #[error(transparent)]
    Mail(#[from] MailError),
    #[error(transparent)]
    Google(#[from] auth::GoogleSetupError),
    #[error("files.storage is \"s3\": add a [files.s3] section with bucket, region and the keys")]
    FilesS3Missing,
    #[error("cannot open the file storage: {0}")]
    Files(#[from] files::FilesError),
    #[error("the optional modules cannot be started: {0}")]
    Modules(#[from] module_api::OrderError),
}

/// Installs the synchronous event subscribers of every layer (ADR-0093). The registry depends only
/// on what is compiled in, so it is the same for every state this process builds (the tests build
/// many), and a second call finds it installed: that is the only error `install` has.
pub fn install_subscribers() {
    let _already_installed = events::install(learning::subscribers());
}

impl AppState {
    /// The composition root of the shared state. It installs the event subscribers and resolves
    /// the editor configuration, so a configuration that does not fit the profiles in code fails
    /// here, at startup, with an error that names the key and the value, and never at the first
    /// request.
    pub fn new(db: PgPool, config: Settings) -> Result<Self, StateError> {
        install_subscribers();
        let editor = Arc::new(EffectiveProfiles::resolve(&config.editor)?);
        let hashing = auth::HashGate::new(config.auth.max_concurrent_hashes);
        let mailer = config
            .mail
            .as_ref()
            .map(|mail| Mailer::new(mail, &config.server.public_origin).map(Arc::new))
            .transpose()?;
        let mail_context = config.mail.as_ref().map(|mail| {
            mail::Context::new(mail, config.institution.language, &config.institution.name)
        });
        let mail_state = MailState::new(db.clone(), &config);
        let google = config
            .auth
            .google
            .as_ref()
            .map(|google| auth::Google::new(google).map(Arc::new))
            .transpose()?;
        let storage = match (&config.files.storage, &config.files.s3) {
            (vumlo_config::FileBackend::Local, _) => {
                files::Storage::local(&config.files.local_dir(&config.base_dir))?
            }
            (vumlo_config::FileBackend::S3, Some(s3)) => {
                files::Storage::s3(s3, s3.access_key.expose(), s3.secret_key.expose())?
            }
            (vumlo_config::FileBackend::S3, None) => return Err(StateError::FilesS3Missing),
        };
        let modules = Arc::new(crate::modules::ordered()?);
        let config = Arc::new(config);
        Ok(Self {
            auth: AuthState {
                db: db.clone(),
                config: config.clone(),
                hashing: Arc::new(hashing),
                google,
            },
            mail: mail_state,
            mailer,
            mail_context,
            org: OrgState {
                db: db.clone(),
                config: config.clone(),
            },
            teaching: TeachingState { db: db.clone() },
            content: ContentState {
                db: db.clone(),
                editor: editor.clone(),
            },
            assessment: AssessmentState { db: db.clone() },
            gradebook: GradebookState {
                db: db.clone(),
                grading_scale: config.institution.grading_scale,
            },
            learning: LearningState {
                db: db.clone(),
                config: config.clone(),
                editor,
                files: storage,
                max_file_bytes: config.files.max_size,
            },
            plugins: PluginsState::new(db.clone(), &config),
            db,
            config,
            modules,
        })
    }

    /// The outbox handlers of every module (ADR-0045), for `events::run_worker` and for tests that
    /// deliver by hand with `events::process_once`.
    pub fn event_handlers(&self) -> Handlers {
        let mut handlers = Handlers::default();
        if self.config.mail.is_some() {
            handlers = handlers.merge(auth::event_handlers(
                self.db.clone(),
                auth::MailContext::new(&self.config),
            ));
        }
        // Without `plugins.secret_key` no plugin can be installed, so there is nobody to owe.
        if self.plugins.key.is_some() {
            handlers = handlers.merge(plugins::event_handlers(self.db.clone()));
        }
        handlers = handlers.merge(learning::event_handlers(self.db.clone()));
        handlers.merge(crate::modules::event_handlers(self))
    }

    /// The mail worker (ADR-0084), to run until the process ends; `None` without `[mail]`.
    pub fn mail_worker(&self) -> Option<impl Future<Output = ()> + Send + 'static> {
        let (mailer, context) = (self.mailer.clone()?, self.mail_context.clone()?);
        Some(mail::run_worker(self.db.clone(), mailer, context))
    }

    /// Sends the events plugins are owed until the process ends. `None` while
    /// `plugins.secret_key` is not set (ADR-0066).
    pub fn plugin_deliverer(&self) -> Result<Option<plugins::Deliverer>, plugins::PluginsError> {
        self.plugins
            .key
            .as_deref()
            .map(|key| {
                plugins::Deliverer::new(
                    key.clone(),
                    self.config.plugins.call_log_days,
                    self.config.plugins.tombstone_days,
                )
            })
            .transpose()
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
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new(auth::COOKIE_NAME))),
        );
        // The admin console's session (ADR-0095): administration needs it, and it is the only
        // credential the console origin sends.
        components.add_security_scheme(
            "console",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
                auth::CONSOLE_COOKIE_NAME,
                "The session of the admin console, from `POST /console/login`.",
            ))),
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
        components.add_security_scheme(
            "plugin",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some(
                        "A plugin token (`vplg_...`) from `vumlo-cli plugin install` (ADR-0066).",
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
    let identity = auth::router(&state.config)?;
    let (router, openapi) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .nest(
            "/api/v1",
            OpenApiRouter::new()
                .routes(routes!(health::health))
                .routes(routes!(health::meta))
                .merge(identity)
                .merge(mail::router())
                .merge(teaching::router())
                .merge(learning::courses::router())
                .merge(learning::items::router())
                .merge(learning::submissions::router())
                .merge(learning::people::router())
                .merge(learning::groups::router())
                .merge(learning::reports::router())
                .merge(content::router())
                .merge(gradebook::router())
                .merge(assessment::router())
                .merge(org::router())
                .merge(learning::audit::router())
                .merge(
                    plugins::router(&state.plugins, &state.config)
                        .map_err(|_| SetupError::RateLimit)?,
                ),
        )
        .split_for_parts();
    let (router, mut openapi) = mount_modules(&state, router, openapi);
    // The files routes carry whole files, so they get their own limits and skip the layers that
    // buffer a body (idempotency, conditional); they are merged in below, after those layers.
    let (files_router, files_doc) = OpenApiRouter::<AppState>::new()
        .nest("/api/v1", learning::files::router())
        .split_for_parts();
    openapi.merge(files_doc);
    let files_router = files_router
        .with_state(state.clone())
        .layer(RequestBodyLimitLayer::new(
            usize::try_from(state.learning.max_file_bytes).unwrap_or(usize::MAX),
        ))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            FILE_REQUEST_TIMEOUT,
        ));
    // The calls people make to a plugin's own server: a plugin may be sent more and take longer
    // than the rest of the API, and an answer is not a document, so these skip the layers that
    // buffer and tag one.
    openapi.merge(plugins::proxy_openapi());
    let proxy_router = Router::<AppState>::new()
        .nest("/api/v1", plugins::proxy_router())
        .with_state(state.clone())
        .layer(RequestBodyLimitLayer::new(plugins::PROXY_MAX_REQUEST_BYTES))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            PROXY_REQUEST_TIMEOUT,
        ));
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
        .merge(files_router)
        .merge(proxy_router)
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

/// Mounts every module that is compiled in and enabled under `/api/v1/modules/<id>`, and adds its
/// paths to the OpenAPI document. A module that is off gets nothing: its paths answer `404` and
/// are not in the document (ADR-0054, ADR-0066).
fn mount_modules(
    state: &AppState,
    mut router: Router<AppState>,
    mut openapi: utoipa::openapi::OpenApi,
) -> (Router<AppState>, utoipa::openapi::OpenApi) {
    for one in crate::modules::active(state) {
        let prefix = format!("/api/v1/modules/{}", one.module.manifest().id);
        let (routes, mut doc) = one.registered.routes.split_for_parts();
        router = router.nest_service(&prefix, routes.with_state(one.state));
        doc.paths.paths = std::mem::take(&mut doc.paths.paths)
            .into_iter()
            .map(|(path, item)| (format!("{prefix}{path}"), item))
            .collect();
        openapi.merge(doc);
    }
    // The core has routes of its own below `/api/v1/modules/` (the modules of a course, whose ids
    // are UUIDs, so they never clash with a module id). A module that is off still answers `404`
    // there, not the `405` of a core route that happens to match the shape of its path.
    for module in state.modules.iter() {
        let id = module.manifest().id;
        if !state.config.module_state.is_enabled(id) {
            router = router.nest_service(
                &format!("/api/v1/modules/{id}"),
                Router::new().fallback(|| async { ApiError::NotFound }),
            );
        }
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
