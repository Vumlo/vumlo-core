mod common;

use std::time::Duration;

use api_types::{HealthResponse, ProblemDetails};
use common::spawn;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

fn content_type(response: &reqwest::Response) -> &str {
    response.headers()["content-type"]
        .to_str()
        .expect("ascii content-type")
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn health_returns_ok(pool: PgPool) {
    let base = spawn(pool).await;

    let response = reqwest::get(format!("{base}/api/v1/health")).await.unwrap();

    assert_eq!(response.status(), 200);
    let body: HealthResponse = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(
        body,
        HealthResponse {
            status: "ok".into(),
            database: "ok".into()
        }
    );
}

#[tokio::test]
async fn health_returns_503_problem_when_database_is_unreachable() {
    // Port 1 is closed, so every connection attempt is refused.
    let pool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_secs(1))
        .connect_lazy("postgres://learnly:learnly@127.0.0.1:1/learnly")
        .unwrap();
    let base = spawn(pool).await;

    let response = reqwest::get(format!("{base}/api/v1/health")).await.unwrap();

    assert_eq!(response.status(), 503);
    assert_eq!(content_type(&response), "application/problem+json");
    let problem: ProblemDetails = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(problem.status, 503);
    assert_eq!(problem.detail.as_deref(), Some("database unavailable"));
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn unknown_route_returns_404_problem(pool: PgPool) {
    let base = spawn(pool).await;

    let response = reqwest::get(format!("{base}/api/v1/nope")).await.unwrap();

    assert_eq!(response.status(), 404);
    assert_eq!(content_type(&response), "application/problem+json");
    let problem: ProblemDetails = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(problem.status, 404);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn framework_generated_405_is_a_problem(pool: PgPool) {
    let base = spawn(pool).await;

    let response = reqwest::Client::new()
        .post(format!("{base}/api/v1/health"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 405);
    assert_eq!(content_type(&response), "application/problem+json");
    let problem: ProblemDetails = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(problem.status, 405);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn openapi_document_contains_health_path(pool: PgPool) {
    let base = spawn(pool).await;

    let response = reqwest::get(format!("{base}/api/v1/openapi.json"))
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let doc: serde_json::Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert!(doc["openapi"].as_str().unwrap().starts_with("3."));
    assert!(doc["paths"]["/api/v1/health"]["get"].is_object(), "{doc}");
    assert!(
        doc["components"]["schemas"]["HealthResponse"].is_object(),
        "{doc}"
    );
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn openapi_document_declares_cookie_and_bearer_security(pool: PgPool) {
    let base = spawn(pool).await;

    let doc: serde_json::Value = serde_json::from_slice(
        &reqwest::get(format!("{base}/api/v1/openapi.json"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
    )
    .unwrap();

    let schemes = &doc["components"]["securitySchemes"];
    assert_eq!(schemes["cookie"]["type"], "apiKey");
    assert_eq!(schemes["cookie"]["in"], "cookie");
    assert_eq!(schemes["cookie"]["name"], "learnly_session");
    assert_eq!(schemes["bearer"]["type"], "http");
    assert_eq!(schemes["bearer"]["scheme"], "bearer");
    let logout = &doc["paths"]["/api/v1/auth/logout"]["post"];
    assert_eq!(
        logout["security"][0]["cookie"],
        serde_json::json!([]),
        "{logout}"
    );
    assert_eq!(
        logout["security"][1]["bearer"],
        serde_json::json!([]),
        "{logout}"
    );
    assert!(
        doc["paths"]["/api/v1/auth/login"]["post"].is_object(),
        "{doc}"
    );
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn docs_page_lists_health_endpoint(pool: PgPool) {
    let base = spawn(pool).await;

    let response = reqwest::get(format!("{base}/api/v1/docs")).await.unwrap();

    assert_eq!(response.status(), 200);
    assert!(content_type(&response).starts_with("text/html"));
    assert!(response.text().await.unwrap().contains("/api/v1/health"));
}
