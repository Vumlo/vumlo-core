// Each test binary compiles this module and uses only some helpers.
#![allow(dead_code)]

use std::sync::Arc;

use server::app::{self, AppState};
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
        app::router(state),
        std::future::pending(),
    ));
    format!("http://{addr}")
}
