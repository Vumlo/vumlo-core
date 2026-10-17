// Each test binary compiles this module and uses only some helpers.
#![allow(dead_code)]

use server::app::{self, AppState};
use server::identity::{self, Email, Role, TemporaryPassword};
use server::settings::{
    AuthSettings, DatabaseSettings, EditorSettings, GradingScale, InstitutionMode,
    InstitutionSettings, LogFormat, LogSettings, ServerSettings, Settings,
};
use sqlx::PgPool;
use tokio::net::TcpListener;

pub const ORIGIN: &str = "https://lms.test";

pub fn test_settings() -> Settings {
    Settings {
        server: ServerSettings {
            bind_addr: ([127, 0, 0, 1], 0).into(),
            public_origin: ORIGIN.to_owned(),
            client_ip_header: Some("x-real-ip".to_owned()),
        },
        database: DatabaseSettings {
            url: String::new(),
            max_connections: 1,
        },
        log: LogSettings {
            format: LogFormat::Pretty,
        },
        auth: AuthSettings::default(),
        institution: InstitutionSettings {
            name: "Test University".to_owned(),
            mode: InstitutionMode::University,
            grading_scale: GradingScale::Hundred,
        },
        editor: EditorSettings::default(),
    }
}

/// Starts the real server on a random local port and returns its base URL.
pub async fn spawn(db: PgPool) -> String {
    spawn_with(db, test_settings()).await
}

pub async fn spawn_with(db: PgPool, settings: Settings) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("listener address");
    let state = AppState::new(db, settings).expect("test app state");
    tokio::spawn(app::serve(
        listener,
        app::router(state).expect("test router"),
        std::future::pending(),
    ));
    format!("http://{addr}")
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
        Self(self.0.header("cookie", format!("learnly_session={token}")))
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
/// returns the temporary password. Plain SQL: `identity::create_user` is private so that no
/// production code can skip the audit (ADR-0034), and this writes no audit row either.
pub async fn seed_pending_user(pool: &PgPool, email: &str, role: Role) -> String {
    let temporary = TemporaryPassword::generate();
    let hash = identity::hash_password(temporary.expose())
        .await
        .expect("hash");
    let email = Email::parse(email).expect("email");
    let id = uuid::Uuid::now_v7();
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query!(
        "INSERT INTO identity.users (id, email, name, surname, role, must_change_password)
         VALUES ($1, $2, 'Test', 'User', $3, true)",
        id,
        email.as_str(),
        role.as_str()
    )
    .execute(&mut *tx)
    .await
    .expect("insert user");
    sqlx::query!(
        "INSERT INTO identity.identities (id, user_id, provider, subject, password_hash)
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
    let hash = identity::hash_password(PASSWORD).await.expect("hash");
    sqlx::query!(
        "UPDATE identity.identities SET password_hash = $2
         WHERE subject = (SELECT id::text FROM identity.users WHERE email = $1)",
        email,
        hash.as_str()
    )
    .execute(pool)
    .await
    .expect("set password");
    sqlx::query!(
        "UPDATE identity.users SET must_change_password = false WHERE email = $1",
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
    let id = sqlx::query_scalar!("SELECT id FROM identity.users WHERE email = $1", email)
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
        .strip_prefix("learnly_session=")
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
