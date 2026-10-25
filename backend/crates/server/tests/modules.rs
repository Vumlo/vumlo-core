//! ADR-0054 and ADR-0066: an optional module is mounted under `/api/v1/modules/<id>`, started and
//! listed only while it is compiled in and enabled, signs people in like the core does, checks its
//! own permissions through `access`, and switching it off never touches its data. The module is
//! the test-only `dummy`.
#![cfg(feature = "dummy")]

mod common;

use std::collections::BTreeMap;

use ::common::Role;
use common::{Http, db_test, seed_and_login, spawn_app, test_settings};
use events::Event;
use server::modules;
use sqlx::PgPool;
use uuid::Uuid;
use vumlo_config::{ModuleEntry, Settings, resolve_modules};

fn settings(enabled: bool) -> Settings {
    let mut settings = test_settings();
    settings.modules = BTreeMap::from([("dummy".to_owned(), ModuleEntry::Flag(enabled))]);
    settings.module_state = resolve_modules(&settings.modules, &BTreeMap::new(), &modules::specs())
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
    sqlx::query_scalar("SELECT count(*) FROM mod_dummy.items")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn heartbeats(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM mod_dummy.heartbeat")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn seen_events(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM mod_dummy.seen_events")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_disabled_module_has_no_routes_no_openapi_paths_no_workers_and_is_not_in_meta() {
    db_test(async |pool| {
        migrate_dummy(&pool).await;
        let (base, state) = spawn_app(pool.clone(), settings(false)).await;
        let http = Http::new(&base);

        assert_eq!(http.get("/modules/dummy/items").send().await.status, 404);
        let doc = http.get("/openapi.json").send().await.json();
        assert!(
            doc["paths"].get("/api/v1/modules/dummy/items").is_none(),
            "{doc}"
        );
        assert!(modules::spawn_workers(&state).is_empty());
        assert_eq!(heartbeats(&pool).await, 0);
        assert_eq!(
            http.get("/meta").send().await.json()["modules"],
            serde_json::json!([])
        );
    })
    .await;
}

#[tokio::test]
async fn an_enabled_module_is_listed_in_meta_and_in_the_openapi_document() {
    db_test(async |pool| {
        migrate_dummy(&pool).await;
        let (base, _) = spawn_app(pool.clone(), settings(true)).await;
        let http = Http::new(&base);

        let meta = http.get("/meta").send().await;
        assert_eq!(meta.status, 200);
        assert_eq!(
            meta.json()["modules"],
            serde_json::json!([{"id": "dummy", "version": env!("CARGO_PKG_VERSION")}])
        );
        let doc = http.get("/openapi.json").send().await.json();
        assert!(
            doc["paths"].get("/api/v1/modules/dummy/items").is_some(),
            "{doc}"
        );
        assert!(doc["paths"].get("/api/v1/dummy/items").is_none(), "{doc}");
    })
    .await;
}

#[tokio::test]
async fn a_module_route_needs_a_session_and_its_permission_is_decided_by_access() {
    db_test(async |pool| {
        migrate_dummy(&pool).await;
        let (base, _) = spawn_app(pool.clone(), settings(true)).await;
        let http = Http::new(&base);
        let (_, teacher) = seed_and_login(&pool, &http, "t@lms.test", Role::Teacher).await;
        let (_, student) = seed_and_login(&pool, &http, "s@lms.test", Role::Student).await;
        let (_, admin) = seed_and_login(&pool, &http, "a@lms.test", Role::Admin).await;
        let item = serde_json::json!({"label": "first"});

        // Nobody signed in: 401, as in the core.
        http.get("/modules/dummy/items").send().await.problem(401);
        http.post("/modules/dummy/items")
            .json(&item)
            .send()
            .await
            .problem(401);

        // A student may read, but does not hold `items.write`.
        assert_eq!(
            http.get("/modules/dummy/items")
                .bearer(&student)
                .send()
                .await
                .status,
            200
        );
        http.post("/modules/dummy/items")
            .bearer(&student)
            .json(&item)
            .send()
            .await
            .problem(403);
        assert_eq!(items(&pool).await, 0);

        // A teacher and an admin hold it.
        for token in [&teacher, &admin] {
            let created = http
                .post("/modules/dummy/items")
                .bearer(token)
                .json(&item)
                .send()
                .await;
            assert_eq!(created.status, 201);
        }
        let listed = http
            .get("/modules/dummy/items")
            .bearer(&student)
            .send()
            .await
            .json();
        assert_eq!(listed.as_array().unwrap().len(), 2, "{listed}");

        // A bad body is a 422 on its field.
        let bad = http
            .post("/modules/dummy/items")
            .bearer(&teacher)
            .json(&serde_json::json!({"label": "   "}))
            .send()
            .await
            .problem(422);
        assert_eq!(bad.errors[0].field, "label");
    })
    .await;
}

#[tokio::test]
async fn an_enabled_module_starts_its_job_and_handles_the_events_it_subscribed_to() {
    db_test(async |pool| {
        migrate_dummy(&pool).await;
        let (_, state) = spawn_app(pool.clone(), settings(true)).await;

        assert_eq!(modules::spawn_workers(&state), ["dummy"]);
        common::wait_until(|| async { heartbeats(&pool).await > 0 }).await;

        let mut conn = pool.acquire().await.unwrap();
        events::emit(
            &mut conn,
            Event::SubmissionSubmitted {
                assignment_id: Uuid::now_v7(),
                student_id: Uuid::now_v7(),
                course_id: Uuid::now_v7(),
                submission_id: Uuid::now_v7(),
            },
        )
        .await
        .unwrap();
        drop(conn);
        events::process_once(&pool, &state.event_handlers())
            .await
            .unwrap();
        assert_eq!(seen_events(&pool).await, 1);
        // Delivered again (at least once): the handler tolerates the repeat.
        sqlx::query("UPDATE events.outbox SET processed_at = NULL, run_after = now()")
            .execute(&pool)
            .await
            .unwrap();
        events::process_once(&pool, &state.event_handlers())
            .await
            .unwrap();
        assert_eq!(seen_events(&pool).await, 1);
    })
    .await;
}

#[tokio::test]
async fn switching_a_module_off_keeps_its_data() {
    db_test(async |pool| {
        migrate_dummy(&pool).await;
        let (on, _) = spawn_app(pool.clone(), settings(true)).await;
        let http = Http::new(&on);
        let (_, teacher) = seed_and_login(&pool, &http, "t@lms.test", Role::Teacher).await;
        http.post("/modules/dummy/items")
            .bearer(&teacher)
            .json(&serde_json::json!({"label": "kept"}))
            .send()
            .await;
        assert_eq!(items(&pool).await, 1);

        let (off, _) = spawn_app(pool.clone(), settings(false)).await;

        let gone = Http::new(&off)
            .get("/modules/dummy/items")
            .bearer(&teacher)
            .send()
            .await;
        assert_eq!(gone.status, 404);
        assert_eq!(
            items(&pool).await,
            1,
            "nothing is deleted when a module is off"
        );
    })
    .await;
}

#[tokio::test]
async fn the_module_keeps_its_tables_and_its_migrations_in_its_own_schema() {
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
        assert_eq!(state.table, "mod_dummy._sqlx_migrations");
        assert_eq!(state.applied.len(), 1);
        assert_eq!(db::migrated_modules(&mut conn).await.unwrap(), ["dummy"]);

        // Nothing of the module outside `mod_dummy`.
        let strays: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM information_schema.tables
             WHERE table_schema <> 'mod_dummy' AND table_name IN ('heartbeat', 'seen_events')
                OR (table_schema = 'public' AND table_name = 'items')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(strays, 0);
    })
    .await;
}
