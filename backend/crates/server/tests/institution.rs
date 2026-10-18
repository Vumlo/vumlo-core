//! ADR-0044: the institution profile is read from the configuration, publicly and for admins.

mod common;

use chrono_tz::Tz;
use common::{Http, db_test, seed_and_login, spawn, spawn_with, test_settings};
use server::identity::Role;
use vumlo_config::Language;

#[tokio::test]
async fn the_public_profile_needs_no_session() {
    db_test(async |pool| {
        let http = Http::new(&spawn(pool).await);

        let response = http.get("/institution").send().await;

        assert_eq!(response.status, 200);
        assert_eq!(
            response.json(),
            serde_json::json!({
                "name": "Test University",
                "short_name": "Test",
                "language": "en",
                "time_zone": "UTC",
                "grading_scale": "hundred",
            })
        );
    })
    .await;
}

#[tokio::test]
async fn the_profile_follows_the_configuration() {
    db_test(async |pool| {
        let mut settings = test_settings();
        settings.institution.short_name = "Northfield".to_owned();
        settings.institution.language = Language::Uk;
        settings.institution.time_zone = "Europe/Kyiv".parse::<Tz>().unwrap();
        let http = Http::new(&spawn_with(pool, settings).await);

        let body = http.get("/institution").send().await.json();

        assert_eq!(body["short_name"], "Northfield");
        assert_eq!(body["language"], "uk");
        assert_eq!(body["time_zone"], "Europe/Kyiv");
    })
    .await;
}

#[tokio::test]
async fn an_admin_reads_the_mode_and_the_allowed_domains() {
    db_test(async |pool| {
        let mut settings = test_settings();
        settings.auth.allowed_domains = vec!["a.edu".to_owned(), "b.edu".to_owned()];
        let http = Http::new(&spawn_with(pool.clone(), settings).await);
        let (_, admin) = seed_and_login(&pool, &http, "admin@a.edu", Role::Admin).await;

        let response = http.get("/admin/institution").bearer(&admin).send().await;

        assert_eq!(response.status, 200);
        assert_eq!(
            response.json(),
            serde_json::json!({
                "name": "Test University",
                "short_name": "Test",
                "language": "en",
                "time_zone": "UTC",
                "grading_scale": "hundred",
                "mode": "university",
                "allowed_domains": ["a.edu", "b.edu"],
            })
        );
    })
    .await;
}

#[tokio::test]
async fn only_an_admin_reads_the_admin_view() {
    db_test(async |pool| {
        let http = Http::new(&spawn(pool.clone()).await);
        let (_, teacher) = seed_and_login(&pool, &http, "t@a.edu", Role::Teacher).await;
        let (_, student) = seed_and_login(&pool, &http, "s@a.edu", Role::Student).await;

        for token in [&teacher, &student] {
            let response = http.get("/admin/institution").bearer(token).send().await;
            response.problem(403);
        }
        http.get("/admin/institution").send().await.problem(401);
    })
    .await;
}

#[tokio::test]
async fn there_is_no_way_to_write_the_profile() {
    db_test(async |pool| {
        let http = Http::new(&spawn(pool.clone()).await);
        let (_, admin) = seed_and_login(&pool, &http, "admin@a.edu", Role::Admin).await;

        for path in ["/institution", "/admin/institution"] {
            let response = http
                .patch(path)
                .bearer(&admin)
                .json(&serde_json::json!({"name": "Other"}))
                .send()
                .await;
            assert_eq!(response.status, 405, "{path}");
        }
    })
    .await;
}

#[tokio::test]
async fn the_spec_lists_both_endpoints() {
    db_test(async |pool| {
        let http = Http::new(&spawn(pool).await);

        let doc = http.get("/openapi.json").send().await.json();

        assert!(
            doc["paths"]["/api/v1/institution"]["get"].is_object(),
            "{doc}"
        );
        assert!(
            doc["paths"]["/api/v1/admin/institution"]["get"].is_object(),
            "{doc}"
        );
        assert!(doc["paths"]["/api/v1/admin/institution"]["patch"].is_null());
    })
    .await;
}
