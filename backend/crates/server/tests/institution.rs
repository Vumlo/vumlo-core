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
                "app_downloads": [],
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
                "app_downloads": [],
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

// --- the links to the desktop app (ADR-0064) -------------------------------------------------------

const WINDOWS: &str = "https://downloads.test/app-setup.exe";
const MAC: &str = "https://downloads.test/app-arm.dmg";

async fn set_links(http: &Http, token: &str, links: serde_json::Value) -> common::Resp {
    http.put("/admin/institution/app-downloads")
        .bearer(token)
        .json(&serde_json::json!({"downloads": links}))
        .send()
        .await
}

#[tokio::test]
async fn an_admin_sets_the_download_links_and_everyone_sees_them_before_signing_in() {
    db_test(async |pool| {
        let http = Http::new(&spawn(pool.clone()).await);
        let (_, admin) = seed_and_login(&pool, &http, "admin@a.edu", Role::Admin).await;

        let set = set_links(
            &http,
            &admin,
            serde_json::json!([
                {"platform": "macos_arm", "url": MAC},
                {"platform": "windows", "url": WINDOWS},
            ]),
        )
        .await;
        assert_eq!(set.status, 200, "{}", String::from_utf8_lossy(&set.body));
        let expected = serde_json::json!([
            {"platform": "windows", "url": WINDOWS},
            {"platform": "macos_arm", "url": MAC},
        ]);
        assert_eq!(
            set.json(),
            expected,
            "in the order the platforms are offered"
        );

        assert_eq!(
            http.get("/institution").send().await.json()["app_downloads"],
            expected
        );
        let admin_view = http
            .get("/admin/institution")
            .bearer(&admin)
            .send()
            .await
            .json();
        assert_eq!(admin_view["app_downloads"], expected);
    })
    .await;
}

#[tokio::test]
async fn the_list_is_the_whole_set_so_a_missing_platform_is_removed_and_an_empty_list_switches_it_off()
 {
    db_test(async |pool| {
        let http = Http::new(&spawn(pool.clone()).await);
        let (_, admin) = seed_and_login(&pool, &http, "admin@a.edu", Role::Admin).await;
        set_links(&http, &admin, serde_json::json!([{"platform": "windows", "url": WINDOWS}, {"platform": "linux_deb", "url": "https://downloads.test/app.deb"}])).await;

        let changed = set_links(&http, &admin, serde_json::json!([{"platform": "windows", "url": "https://downloads.test/new.exe"}])).await;
        assert_eq!(changed.json(), serde_json::json!([{"platform": "windows", "url": "https://downloads.test/new.exe"}]));

        assert_eq!(set_links(&http, &admin, serde_json::json!([])).await.status, 200);
        assert_eq!(http.get("/institution").send().await.json()["app_downloads"], serde_json::json!([]));
    })
    .await;
}

#[tokio::test]
async fn only_an_admin_sets_the_links_and_a_session_is_needed() {
    db_test(async |pool| {
        let http = Http::new(&spawn(pool.clone()).await);
        let (_, teacher) = seed_and_login(&pool, &http, "teacher@a.edu", Role::Teacher).await;
        let (_, student) = seed_and_login(&pool, &http, "student@a.edu", Role::Student).await;
        let body = serde_json::json!([{"platform": "windows", "url": WINDOWS}]);

        assert_eq!(set_links(&http, &teacher, body.clone()).await.status, 403);
        assert_eq!(set_links(&http, &student, body.clone()).await.status, 403);
        let anonymous = http
            .put("/admin/institution/app-downloads")
            .json(&serde_json::json!({"downloads": body}))
            .send()
            .await;
        assert_eq!(anonymous.status, 401);
        assert_eq!(
            http.get("/institution").send().await.json()["app_downloads"],
            serde_json::json!([])
        );
    })
    .await;
}

#[tokio::test]
async fn a_link_that_is_not_an_https_address_is_refused_and_nothing_changes() {
    db_test(async |pool| {
        let http = Http::new(&spawn(pool.clone()).await);
        let (_, admin) = seed_and_login(&pool, &http, "admin@a.edu", Role::Admin).await;
        set_links(&http, &admin, serde_json::json!([{"platform": "windows", "url": WINDOWS}])).await;
        let too_long = format!("https://downloads.test/{}", "a".repeat(2048));

        for bad in [
            "http://downloads.test/a.exe",
            "javascript:alert(1)",
            "//downloads.test/a.exe",
            "https://",
            "https:///a.exe",
            "https://user@downloads.test/a.exe",
            "https://downloads.test/a b.exe",
            "https://downloads.test/a\nb",
            "downloads.test/a.exe",
            "",
            too_long.as_str(),
        ] {
            let response = set_links(&http, &admin, serde_json::json!([{"platform": "linux_deb", "url": bad}])).await;
            assert_eq!(response.status, 422, "{bad:?}");
            assert_eq!(response.problem(422).errors[0].field, "downloads", "{bad:?}");
        }
        let twice = set_links(&http, &admin, serde_json::json!([{"platform": "windows", "url": WINDOWS}, {"platform": "windows", "url": MAC}])).await;
        assert_eq!(twice.status, 422);
        let unknown = set_links(&http, &admin, serde_json::json!([{"platform": "amiga", "url": WINDOWS}])).await;
        assert_eq!(unknown.status, 422);
        let many: Vec<_> = (0..6).map(|_| serde_json::json!({"platform": "windows", "url": WINDOWS})).collect();
        assert_eq!(set_links(&http, &admin, serde_json::Value::Array(many)).await.status, 422);

        assert_eq!(
            http.get("/institution").send().await.json()["app_downloads"],
            serde_json::json!([{"platform": "windows", "url": WINDOWS}]),
            "the refusals left the links as they were"
        );
    })
    .await;
}

#[tokio::test]
async fn setting_the_links_is_audited_with_the_platforms_and_not_the_addresses() {
    db_test(async |pool| {
        let http = Http::new(&spawn(pool.clone()).await);
        let (admin_id, admin) = seed_and_login(&pool, &http, "admin@a.edu", Role::Admin).await;
        set_links(&http, &admin, serde_json::json!([{"platform": "windows", "url": WINDOWS}, {"platform": "macos_arm", "url": MAC}])).await;

        let row = sqlx::query!(
            r#"SELECT actor_id, source, resource_type, meta FROM events.audit_log WHERE action = 'institution.app_downloads.set'"#
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.actor_id, Some(admin_id));
        assert_eq!(row.resource_type, "institution");
        assert_eq!(row.meta, serde_json::json!({"platforms": ["windows", "macos_arm"]}));
        assert!(!row.meta.to_string().contains("downloads.test"));
        let _ = row.source;
    })
    .await;
}
