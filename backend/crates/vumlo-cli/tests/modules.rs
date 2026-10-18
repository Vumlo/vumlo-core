//! ADR-0054 in the CLI: a module that is compiled in migrates into its own table, has a
//! subcommand, and is reported in all three states. The module is the test-only `dummy`.
#![cfg(feature = "dummy")]

mod common;

use common::{CONFIG, Env, audit, audit_meta, bare_db_test};
use serde_json::{Value, json};

fn with_modules(extra: &str) -> String {
    format!("{CONFIG}\n[modules]\n{extra}\n")
}

#[tokio::test]
async fn migrate_applies_the_module_into_its_own_table_and_records_it_in_the_audit_row() {
    bare_db_test(async |pool| {
        let env = Env::with_config(&pool, &with_modules("dummy = false"));

        let out = env.run(&["db", "migrate"]).await;

        assert!(out.ok, "{}", out.stderr);
        let meta = audit_meta(&pool, "db.migrate").await;
        assert_eq!(meta["applied"]["dummy"], json!([1]), "{meta}");
        let tables: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_tables WHERE tablename = '_sqlx_migrations_dummy'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(tables, 1);
        assert_eq!(audit(&pool).await.len(), 1);

        let again = env.run(&["db", "migrate"]).await;
        assert!(again.ok && again.stdout.contains("up to date"));
        assert_eq!(audit(&pool).await.len(), 1);
    })
    .await;
}

#[tokio::test]
async fn config_reports_a_module_as_enabled_disabled_or_unavailable() {
    bare_db_test(async |pool| {
        let env = Env::with_config(&pool, &with_modules("dummy = true"));
        let out = env.run(&["config", "show", "--public"]).await;
        assert!(out.ok, "{}", out.stderr);
        let value: Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(value["modules"]["dummy"], "enabled");
        assert_eq!(value["modules"]["dummy-absent"], "unavailable");
        assert!(
            !out.stdout.contains("endpoint"),
            "module settings never reach the client"
        );

        let off = Env::with_config(&pool, &with_modules("dummy = false"));
        let value: Value =
            serde_json::from_str(&off.run(&["config", "show", "--public"]).await.stdout).unwrap();
        assert_eq!(value["modules"]["dummy"], "disabled");
    })
    .await;
}

#[tokio::test]
async fn enabling_a_module_that_is_not_compiled_in_says_which_feature_to_build_with() {
    bare_db_test(async |pool| {
        let env = Env::with_config(&pool, &with_modules("dummy = true\ndummy-absent = true"));

        let check = env.run(&["config", "check"]).await;
        assert!(!check.ok);
        assert!(
            check
                .stderr
                .contains("build with the Cargo feature `dummy-absent`"),
            "{}",
            check.stderr
        );

        // Doctor carries on and warns instead.
        let doctor = env.run(&["doctor", "--json"]).await;
        let r: Value = serde_json::from_str(&doctor.stdout).unwrap();
        let line = r["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "module dummy-absent")
            .unwrap_or_else(|| panic!("{r:#}"));
        assert_eq!(line["status"], "warn");
    })
    .await;
}

#[tokio::test]
async fn a_module_needs_the_modules_it_depends_on() {
    bare_db_test(async |pool| {
        let env = Env::with_config(&pool, &with_modules("dummy-absent = true"));
        let out = env.run(&["config", "check"]).await;
        assert!(!out.ok);
        assert!(out.stderr.contains("needs modules.dummy"), "{}", out.stderr);
    })
    .await;
}

#[tokio::test]
async fn a_module_secret_in_the_file_is_refused_and_one_in_the_environment_is_used() {
    bare_db_test(async |pool| {
        let in_file = Env::with_config(
            &pool,
            &format!("{CONFIG}\n[modules.dummy]\nenabled = true\napi_key = \"in-the-file\"\n"),
        );
        let out = in_file.run(&["config", "check"]).await;
        assert!(!out.ok);
        assert!(out.stderr.contains("VUMLO_DUMMY_API_KEY"), "{}", out.stderr);
        assert!(!out.stderr.contains("in-the-file"));

        let ok = Env::with_config(
            &pool,
            &format!("{CONFIG}\n[modules.dummy]\nenabled = true\n"),
        );
        let out = ok
            .run_with(
                &["config", "show"],
                None,
                &[
                    ("DATABASE_URL", ok.url.as_str()),
                    ("VUMLO_DUMMY_API_KEY", "sentinel-key"),
                ],
            )
            .await;
        assert!(
            out.ok && !out.stdout.contains("sentinel-key"),
            "{}",
            out.stderr
        );
    })
    .await;
}

#[tokio::test]
async fn the_module_subcommand_runs_only_while_the_module_is_enabled() {
    bare_db_test(async |pool| {
        let on = Env::with_config(&pool, &with_modules("dummy = true"));
        assert!(on.run(&["db", "migrate"]).await.ok);
        sqlx::query("INSERT INTO dummy.items (id, label) VALUES (gen_random_uuid(), 'x')")
            .execute(&pool)
            .await
            .unwrap();

        let count = on.run(&["module", "dummy", "count"]).await;
        assert!(count.ok, "{}", count.stderr);
        assert_eq!(count.stdout.trim(), "1");

        let off = Env::with_config(&pool, &with_modules("dummy = false"));
        let refused = off.run(&["module", "dummy", "count"]).await;
        assert!(!refused.ok);
        assert!(refused.stderr.contains("disabled"), "{}", refused.stderr);
    })
    .await;
}

#[tokio::test]
async fn doctor_warns_about_data_of_a_disabled_module_and_flags_a_missing_secret() {
    bare_db_test(async |pool| {
        let off = Env::with_config(&pool, &with_modules("dummy = false"));
        assert!(off.run(&["db", "migrate"]).await.ok);
        sqlx::query("INSERT INTO dummy.items (id, label) VALUES (gen_random_uuid(), 'x')")
            .execute(&pool)
            .await
            .unwrap();
        let find = |r: &Value, name: &str| -> Value {
            r["checks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["name"] == name)
                .cloned()
                .unwrap_or_else(|| panic!("no {name} in {r:#}"))
        };

        let r: Value = serde_json::from_str(&off.run(&["doctor", "--json"]).await.stdout).unwrap();
        let data = find(&r, "dummy data");
        assert_eq!(data["status"], "warn");
        assert!(data["detail"].as_str().unwrap().contains("never deleted"));

        let on = Env::with_config(
            &pool,
            &format!(
                "{CONFIG}\n[modules.dummy]\nenabled = true\nendpoint = \"https://api.example.invalid\"\n"
            ),
        );
        let r: Value = serde_json::from_str(&on.run(&["doctor", "--json"]).await.stdout).unwrap();
        let endpoint = find(&r, "dummy endpoint");
        assert_eq!(endpoint["status"], "fail");
        assert!(endpoint["detail"].as_str().unwrap().contains("VUMLO_DUMMY_API_KEY"));
    })
    .await;
}
