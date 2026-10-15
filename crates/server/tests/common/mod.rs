// Each test binary compiles this module and uses only some helpers.
#![allow(dead_code)]

use std::sync::Arc;

use server::app::{self, AppState};
use server::identity::{self, DisplayName, Email, NewUser, Role, TemporaryPassword};
use server::settings::{
    AuthSettings, DatabaseSettings, InstitutionSettings, LogFormat, LogSettings, ServerSettings,
    Settings,
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
        },
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
    let state = AppState {
        db,
        config: Arc::new(settings),
    };
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
/// returns the temporary password.
pub async fn seed_pending_user(pool: &PgPool, email: &str, role: Role) -> String {
    let temporary = TemporaryPassword::generate();
    let hash = identity::hash_password(temporary.expose())
        .await
        .expect("hash");
    let mut tx = pool.begin().await.expect("begin");
    identity::create_user(
        &mut tx,
        NewUser {
            email: Email::parse(email).expect("email"),
            display_name: DisplayName::parse("Test User").expect("name"),
            role,
        },
        hash,
    )
    .await
    .expect("create user");
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
