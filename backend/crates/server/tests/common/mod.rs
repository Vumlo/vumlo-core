// Each test binary compiles this module and uses only some helpers.
#![allow(dead_code)]

pub mod google;

use ::common::Role;
use server::app::{self, AppState};
use server::auth::{self, Email, TemporaryPassword};
use sqlx::PgPool;
use tokio::net::TcpListener;
use vumlo_config::{
    AuthSettings, CliSettings, DatabaseSettings, DesktopSettings, EditorSettings, GradingScale,
    InstitutionMode, InstitutionSettings, Language, LogFormat, LogSettings, Modules, Secret,
    ServerSettings, Settings, WebSettings,
};

// The per-test database harness (ADR-0036).
// Not every test binary needs the bare variant.
#[allow(unused_imports)]
pub use test_support::{bare_db_test, db_test, wait_until_blocked};

pub const ORIGIN: &str = "https://lms.test";

/// Applies the migrations of the optional modules compiled in. `db_test` only migrates the core,
/// and a binary built with a module treats a database with its migrations pending as not ready.
pub async fn migrate_modules(pool: &PgPool) {
    for (_, migrator) in server::modules::migrators() {
        migrator.run(pool).await.expect("the module's migrations");
    }
}

/// Polls `check` until it holds, for work that happens on its own task. Fails after ten seconds.
pub async fn wait_until<F, Fut>(mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !check().await {
        assert!(
            std::time::Instant::now() < deadline,
            "the condition did not hold within ten seconds"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

pub fn test_settings() -> Settings {
    Settings {
        config_version: vumlo_config::SUPPORTED_CONFIG_VERSION,
        server: ServerSettings {
            bind_addr: ([127, 0, 0, 1], 0).into(),
            public_origin: ORIGIN.to_owned(),
            client_ip_header: Some("x-real-ip".to_owned()),
            allowed_origins: Vec::new(),
        },
        database: DatabaseSettings {
            url: Secret::new(""),
            max_connections: 1,
        },
        web: WebSettings::default(),
        desktop: DesktopSettings::default(),
        cli: CliSettings::default(),
        log: LogSettings {
            format: LogFormat::Pretty,
        },
        auth: AuthSettings::default(),
        institution: InstitutionSettings {
            name: "Test University".to_owned(),
            short_name: "Test".to_owned(),
            language: Language::En,
            time_zone: chrono_tz::UTC,
            mode: InstitutionMode::University,
            grading_scale: GradingScale::Hundred,
            logo: None,
        },
        editor: EditorSettings::default(),
        mail: None,
        modules: std::collections::BTreeMap::new(),
        module_state: Modules::default(),
        base_dir: std::path::PathBuf::new(),
    }
}

/// Starts the real server on a random local port and returns its base URL.
pub async fn spawn(db: PgPool) -> String {
    spawn_with(db, test_settings()).await
}

pub async fn spawn_with(db: PgPool, settings: Settings) -> String {
    spawn_app(db, settings).await.0
}

/// Like [`spawn_with`], and also returns the state the server runs on, so a test can deliver the
/// outbox by hand with `events::process_once(&state.db, &state.event_handlers())`.
pub async fn spawn_app(db: PgPool, settings: Settings) -> (String, AppState) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("listener address");
    let state = AppState::new(db, settings).expect("test app state");
    tokio::spawn(app::serve(
        listener,
        app::router(state.clone()).expect("test router"),
        std::future::pending(),
    ));
    (format!("http://{addr}"), state)
}

/// A directory the file mail transport writes to (ADR-0045). Removed when dropped.
pub struct Mailbox {
    dir: std::path::PathBuf,
}

impl Mailbox {
    pub fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("vumlo-mailbox-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).expect("mailbox directory");
        Self { dir }
    }

    /// Settings that send mail to this directory and let people sign up from `lms.test`.
    pub fn settings(&self) -> Settings {
        let mut settings = test_settings();
        settings.auth.allowed_domains = vec!["lms.test".to_owned()];
        settings.mail = Some(vumlo_config::MailSettings {
            from: "Vumlo <no-reply@lms.test>".to_owned(),
            link_base: None,
            transport: vumlo_config::MailTransport::File {
                dir: self.dir.clone(),
            },
        });
        settings
    }

    /// The text of every message so far, oldest first, with quoted-printable undone.
    pub fn messages(&self) -> Vec<String> {
        let mut files: Vec<_> = std::fs::read_dir(&self.dir)
            .expect("read mailbox")
            .map(|entry| entry.expect("entry").path())
            .collect();
        files.sort_by_key(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok());
        files
            .into_iter()
            .map(|path| {
                std::fs::read_to_string(path)
                    .expect("read message")
                    .replace("=\r\n", "")
                    .replace("=\n", "")
                    .replace("=3D", "=")
            })
            .collect()
    }

    /// The `token=` value of the last link in `message`.
    pub fn token_in(message: &str) -> String {
        let start = message.rfind("token=").expect("a link with a token") + "token=".len();
        message[start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect()
    }
}

impl Drop for Mailbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ---------------------------------------------------------------------------------------------
// A tiny black-box HTTP client. reqwest's `json` feature is not enabled (ADR-0014).
// ---------------------------------------------------------------------------------------------

pub struct Http {
    base: String,
    client: reqwest::Client,
}

pub struct Req(reqwest::RequestBuilder);

pub struct Resp {
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub body: Vec<u8>,
}

impl Http {
    pub fn new(base: &str) -> Self {
        Self {
            base: base.to_owned(),
            client: reqwest::Client::new(),
        }
    }

    pub fn get(&self, path: &str) -> Req {
        Req(self.client.get(format!("{}/api/v1{path}", self.base)))
    }

    pub fn post(&self, path: &str) -> Req {
        Req(self.client.post(format!("{}/api/v1{path}", self.base)))
    }

    pub fn options(&self, path: &str) -> Req {
        Req(self.client.request(
            reqwest::Method::OPTIONS,
            format!("{}/api/v1{path}", self.base),
        ))
    }

    pub fn put(&self, path: &str) -> Req {
        Req(self.client.put(format!("{}/api/v1{path}", self.base)))
    }

    pub fn patch(&self, path: &str) -> Req {
        Req(self.client.patch(format!("{}/api/v1{path}", self.base)))
    }

    pub fn delete(&self, path: &str) -> Req {
        Req(self.client.delete(format!("{}/api/v1{path}", self.base)))
    }
}

impl Req {
    pub fn bearer(self, token: &str) -> Self {
        Self(self.0.header("authorization", format!("Bearer {token}")))
    }

    pub fn header(self, name: &str, value: &str) -> Self {
        Self(self.0.header(name, value))
    }

    pub fn cookie(self, token: &str) -> Self {
        Self(self.0.header("cookie", format!("vumlo_session={token}")))
    }

    pub fn origin(self, origin: &str) -> Self {
        Self(self.0.header("origin", origin))
    }

    /// The client IP the rate limiter sees (the test config trusts `x-real-ip`).
    pub fn ip(self, ip: &str) -> Self {
        Self(self.0.header("x-real-ip", ip))
    }

    pub fn json(self, body: &serde_json::Value) -> Self {
        Self(
            self.0
                .header("content-type", "application/json")
                .body(body.to_string()),
        )
    }

    pub async fn send(self) -> Resp {
        let response = self.0.send().await.expect("request");
        Resp {
            status: response.status().as_u16(),
            headers: response.headers().clone(),
            body: response.bytes().await.expect("body").to_vec(),
        }
    }
}

impl Resp {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&self.body)))
    }

    pub fn header(&self, name: &str) -> &str {
        self.headers[name].to_str().expect("ascii header")
    }

    /// Asserts a Problem Details body with this status and returns it.
    pub fn problem(&self, status: u16) -> api_types::ProblemDetails {
        assert_eq!(
            self.status,
            status,
            "{}",
            String::from_utf8_lossy(&self.body)
        );
        assert_eq!(self.header("content-type"), "application/problem+json");
        let problem: api_types::ProblemDetails =
            serde_json::from_slice(&self.body).expect("problem details");
        assert_eq!(problem.status, status);
        problem
    }
}

// ---------------------------------------------------------------------------------------------
// Seeding and login helpers
// ---------------------------------------------------------------------------------------------

pub const PASSWORD: &str = "correct horse battery staple";

/// Creates a user the way the CLI does (temporary password, `must_change_password = true`) and
/// returns the temporary password. Plain SQL: `auth::create_user` is private so that no
/// production code can skip the audit (ADR-0034), and this writes no audit row either.
pub async fn seed_pending_user(pool: &PgPool, email: &str, role: Role) -> String {
    let temporary = TemporaryPassword::generate();
    let hash = auth::hash_password(temporary.expose()).await.expect("hash");
    let email = Email::parse(email).expect("email");
    let id = uuid::Uuid::now_v7();
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query!(
        "INSERT INTO auth.users
             (id, email, name, surname, role, must_change_password, email_verified_at)
         VALUES ($1, $2, 'Test', 'User', $3, true, now())",
        id,
        email.as_str(),
        role.as_str()
    )
    .execute(&mut *tx)
    .await
    .expect("insert user");
    sqlx::query!(
        "INSERT INTO auth.identities (id, user_id, provider, subject, password_hash)
         VALUES ($1, $2, 'password', $3, $4)",
        uuid::Uuid::now_v7(),
        id,
        id.to_string(),
        hash.as_str()
    )
    .execute(&mut *tx)
    .await
    .expect("insert identity");
    tx.commit().await.expect("commit");
    temporary.expose().to_owned()
}

/// A user who can act right away: password [`PASSWORD`], no pending change.
pub async fn seed_user(pool: &PgPool, email: &str, role: Role) -> String {
    seed_pending_user(pool, email, role).await;
    let hash = auth::hash_password(PASSWORD).await.expect("hash");
    sqlx::query!(
        "UPDATE auth.identities SET password_hash = $2
         WHERE subject = (SELECT id::text FROM auth.users WHERE email = $1)",
        email,
        hash.as_str()
    )
    .execute(pool)
    .await
    .expect("set password");
    sqlx::query!(
        "UPDATE auth.users SET must_change_password = false WHERE email = $1",
        email
    )
    .execute(pool)
    .await
    .expect("clear flag");
    PASSWORD.to_owned()
}

/// Seeds a ready-to-use user, logs them in as an app client and returns their id and bearer token.
pub async fn seed_and_login(
    pool: &PgPool,
    http: &Http,
    email: &str,
    role: Role,
) -> (uuid::Uuid, String) {
    let password = seed_user(pool, email, role).await;
    let id = sqlx::query_scalar!("SELECT id FROM auth.users WHERE email = $1", email)
        .fetch_one(pool)
        .await
        .expect("user id");
    (id, login_app(http, email, &password).await)
}

/// Logs in as an app client and returns the bearer token.
pub async fn login_app(http: &Http, email: &str, password: &str) -> String {
    let response = http
        .post("/auth/login")
        .json(&serde_json::json!({"email": email, "password": password, "client": "app"}))
        .send()
        .await;
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    response.json()["token"].as_str().expect("token").to_owned()
}

/// Logs in as a web client and returns the session token from the cookie.
pub async fn login_web(http: &Http, email: &str, password: &str) -> String {
    let response = http
        .post("/auth/login")
        .origin(ORIGIN)
        .json(&serde_json::json!({"email": email, "password": password, "client": "web"}))
        .send()
        .await;
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    cookie_token(&response)
}

pub fn cookie_token(response: &Resp) -> String {
    response
        .header("set-cookie")
        .strip_prefix("vumlo_session=")
        .and_then(|rest| rest.split(';').next())
        .expect("session cookie")
        .to_owned()
}

// ---------------------------------------------------------------------------------------------
// Course helpers shared by the course-scoped test files
// ---------------------------------------------------------------------------------------------

pub fn id_of(value: &serde_json::Value) -> uuid::Uuid {
    uuid::Uuid::parse_str(value["id"].as_str().expect("id")).expect("uuid")
}

/// Creates a draft course as the token's user and returns its id.
pub async fn create_course(http: &Http, token: &str, title: &str) -> uuid::Uuid {
    let response = http
        .post("/courses")
        .bearer(token)
        .json(&serde_json::json!({"title": title}))
        .send()
        .await;
    assert_eq!(
        response.status,
        201,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    id_of(&response.json())
}

pub async fn put_member(
    http: &Http,
    token: &str,
    course: uuid::Uuid,
    user: uuid::Uuid,
    role: &str,
) {
    let response = http
        .put(&format!("/courses/{course}/members/{user}"))
        .bearer(token)
        .json(&serde_json::json!({"role": role}))
        .send()
        .await;
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
}

pub async fn publish_course(http: &Http, token: &str, course: uuid::Uuid) {
    let response = http
        .post(&format!("/courses/{course}/publish"))
        .bearer(token)
        .send()
        .await;
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
}

// ---------------------------------------------------------------------------------------------
// A small world for tests that exercise several roles over HTTP
// ---------------------------------------------------------------------------------------------

pub struct Account {
    pub id: uuid::Uuid,
    pub token: String,
}

/// A running server over a test database, with calls that take a bearer token.
pub struct Env {
    pub http: Http,
    pub pool: PgPool,
    pub base: String,
}

impl Env {
    pub async fn new(pool: PgPool) -> Self {
        let base = spawn(pool.clone()).await;
        Self {
            http: Http::new(&base),
            pool,
            base,
        }
    }

    pub async fn with_settings(pool: PgPool, settings: Settings) -> Self {
        let base = spawn_with(pool.clone(), settings).await;
        Self {
            http: Http::new(&base),
            pool,
            base,
        }
    }

    /// Seeds a user who can act right away and logs them in as an app client.
    pub async fn user(&self, email: &str, role: Role) -> Account {
        let (id, token) = seed_and_login(&self.pool, &self.http, email, role).await;
        Account { id, token }
    }

    pub async fn call(
        &self,
        method: &str,
        token: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Resp {
        let request = match method {
            "GET" => self.http.get(path),
            "POST" => self.http.post(path),
            "PUT" => self.http.put(path),
            "PATCH" => self.http.patch(path),
            "DELETE" => self.http.delete(path),
            other => panic!("method {other}"),
        }
        .bearer(token);
        match body {
            Some(body) => request.json(&body).send().await,
            None => request.send().await,
        }
    }

    /// Calls and asserts the status; returns the JSON body (`null` for an empty one).
    pub async fn ok(
        &self,
        method: &str,
        token: &str,
        path: &str,
        body: Option<serde_json::Value>,
        status: u16,
    ) -> serde_json::Value {
        let response = self.call(method, token, path, body).await;
        assert_eq!(
            response.status,
            status,
            "{method} {path}: {}",
            String::from_utf8_lossy(&response.body)
        );
        if response.body.is_empty() {
            serde_json::Value::Null
        } else {
            response.json()
        }
    }

    /// A published course owned by `owner` with `students` enrolled and one published module.
    /// Returns the course and the module.
    pub async fn course(&self, owner: &Account, students: &[&Account]) -> (uuid::Uuid, uuid::Uuid) {
        use serde_json::json;
        let course = id_of(
            &self
                .ok(
                    "POST",
                    &owner.token,
                    "/courses",
                    Some(json!({"title": "C"})),
                    201,
                )
                .await,
        );
        for student in students {
            let path = format!("/courses/{course}/members/{}", student.id);
            self.ok(
                "PUT",
                &owner.token,
                &path,
                Some(json!({"role": "student"})),
                200,
            )
            .await;
        }
        self.ok(
            "POST",
            &owner.token,
            &format!("/courses/{course}/publish"),
            None,
            200,
        )
        .await;
        let path = format!("/courses/{course}/modules");
        let module = id_of(
            &self
                .ok(
                    "POST",
                    &owner.token,
                    &path,
                    Some(json!({"title": "M"})),
                    201,
                )
                .await,
        );
        let path = format!("/modules/{module}/publish");
        self.ok(
            "POST",
            &owner.token,
            &path,
            Some(json!({"cascade": true})),
            200,
        )
        .await;
        (course, module)
    }

    /// A published text assignment worth `points` in `module`.
    pub async fn assignment(
        &self,
        owner: &Account,
        module: uuid::Uuid,
        points: &str,
    ) -> uuid::Uuid {
        use serde_json::json;
        let body = json!({"kind": "assignment", "title": "A",
            "assignment": {"type": "text", "max_points": points}});
        let path = format!("/modules/{module}/items");
        let item = id_of(&self.ok("POST", &owner.token, &path, Some(body), 201).await);
        self.ok(
            "POST",
            &owner.token,
            &format!("/items/{item}/publish"),
            None,
            200,
        )
        .await;
        item
    }
}

/// A document of the student profile with one paragraph.
pub fn essay(text: &str) -> serde_json::Value {
    serde_json::json!({"v": 1, "attachments": [], "blocks": [
        {"id": "p1", "type": "paragraph", "content": [{"type": "text", "text": text}]}]})
}
