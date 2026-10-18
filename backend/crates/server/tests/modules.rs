//! ADR-0054: an optional module is mounted, started and listed only while it is compiled in and
//! enabled, and switching it off never touches its data. The module is the test-only `dummy`.
#![cfg(feature = "dummy")]

mod common;

use std::collections::BTreeMap;

use common::{Http, db_test, spawn_app, test_settings};
use server::modules;
use sqlx::PgPool;
use vumlo_config::{ModuleEntry, Settings, resolve_modules};

fn settings(enabled: bool) -> Settings {
    let mut settings = test_settings();
    settings.modules = BTreeMap::from([("dummy".to_owned(), ModuleEntry::Flag(enabled))]);
    settings.module_state = resolve_modules(&settings.modules, &BTreeMap::new(), &["dummy"])
        .expect("the dummy module resolves");
    settings
}

/// Applies the dummy module's own migrations, as `vumlo-cli db migrate` does.
async fn migrate_dummy(pool: &PgPool) {
    for (_, migrator) in modules::migrators() {
        migrator.run(pool).await.expect("the module's migrations");
    }
}

async fn items(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM dummy.items")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn heartbeats(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM dummy.heartbeat")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_disabled_module_has_no_routes_no_openapi_paths_and_no_workers() {
    db_test(async |pool| {
        migrate_dummy(&pool).await;
        let (base, state) = spawn_app(pool.clone(), settings(false)).await;
        let http = Http::new(&base);

        assert_eq!(http.get("/dummy/items").send().await.status, 404);
        let doc = http.get("/openapi.json").send().await.json();
        assert!(doc["paths"].get("/api/v1/dummy/items").is_none(), "{doc}");
        assert!(modules::spawn_workers(&state).is_empty());
        assert_eq!(heartbeats(&pool).await, 0);
    })
    .await;
}

#[tokio::test]
async fn an_enabled_module_serves_its_routes_lists_its_paths_and_starts_its_worker() {
    db_test(async |pool| {
        migrate_dummy(&pool).await;
        let (base, state) = spawn_app(pool.clone(), settings(true)).await;
        let http = Http::new(&base);

        let created = http
            .post("/dummy/items")
            .json(&serde_json::json!({"label": "first"}))
            .send()
            .await;
        assert_eq!(created.status, 201);
        let listed = http.get("/dummy/items").send().await.json();
        assert_eq!(listed[0]["label"], "first");
        let doc = http.get("/openapi.json").send().await.json();
        assert!(doc["paths"].get("/api/v1/dummy/items").is_some(), "{doc}");

        assert_eq!(modules::spawn_workers(&state), ["dummy"]);
        common::wait_until(|| async { heartbeats(&pool).await > 0 }).await;
    })
    .await;
}

#[tokio::test]
async fn switching_a_module_off_keeps_its_data() {
    db_test(async |pool| {
        migrate_dummy(&pool).await;
        let (on, _) = spawn_app(pool.clone(), settings(true)).await;
        Http::new(&on)
            .post("/dummy/items")
            .json(&serde_json::json!({"label": "kept"}))
            .send()
            .await;
        assert_eq!(items(&pool).await, 1);

        let (off, _) = spawn_app(pool.clone(), settings(false)).await;

        assert_eq!(Http::new(&off).get("/dummy/items").send().await.status, 404);
        assert_eq!(
            items(&pool).await,
            1,
            "nothing is deleted when a module is off"
        );
    })
    .await;
}

#[tokio::test]
async fn the_module_keeps_its_migrations_in_its_own_table() {
    db_test(async |pool| {
        migrate_dummy(&pool).await;

        let mut conn = pool.acquire().await.unwrap();
        let core = db::status(&mut conn, "core", &db::MIGRATOR).await.unwrap();
        assert!(
            core.newer.is_empty(),
            "the core table holds core migrations only"
        );
        let module = modules::migrators().into_iter().next().unwrap();
        let state = db::status(&mut conn, module.0, module.1).await.unwrap();
        assert_eq!(state.table, "_sqlx_migrations_dummy");
        assert_eq!(state.applied.len(), 1);
        assert_eq!(db::migrated_modules(&mut conn).await.unwrap(), ["dummy"]);
    })
    .await;
}
