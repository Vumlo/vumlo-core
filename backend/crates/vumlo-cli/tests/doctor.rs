//! `vumlo-cli doctor`: it reports every check, changes nothing, and fails when one fails.

mod common;

use std::os::unix::fs::PermissionsExt;

use common::{
    CONFIG, Env, Out, audit, bare_db_test, db_test, fake_core, settings_for, spawn_server,
};
use serde_json::{Value, json};

fn report(out: &Out) -> Value {
    serde_json::from_str(&out.stdout)
        .unwrap_or_else(|e| panic!("doctor --json printed {:?}: {e}", out.stdout))
}

/// The status of the check called `name`.
fn status(report: &Value, name: &str) -> String {
    report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no check {name} in {report}"))["status"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn detail(report: &Value, name: &str) -> String {
    report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .unwrap()["detail"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn a_healthy_installation_passes_and_doctor_changes_nothing() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        let core = spawn_server(&pool, settings_for(&env.url, CONFIG, &[])).await;
        let flag = format!("cli.core_url={core}");

        let out = env.run(&["doctor", "--json", "--set", &flag]).await;

        let r = report(&out);
        assert!(out.ok, "{r:#}");
        assert_eq!(r["ok"], true);
        for name in [
            "configuration",
            "environment",
            "database",
            "schema",
            "core",
            "versions",
            "module sets",
        ] {
            assert_eq!(status(&r, name), "ok", "{name}: {r:#}");
        }
        assert!(audit(&pool).await.is_empty(), "doctor is read-only");
    })
    .await;
}

#[tokio::test]
async fn a_missing_database_url_fails_and_the_rest_is_still_reported() {
    let env = Env::at(String::new(), CONFIG);

    let out = env.run_with(&["doctor", "--json"], None, &[]).await;

    let r = report(&out);
    assert!(!out.ok);
    assert_eq!(r["ok"], false);
    assert_eq!(status(&r, "environment"), "fail");
    assert!(detail(&r, "environment").contains("DATABASE_URL"));
    assert_eq!(status(&r, "configuration"), "fail");
    assert_eq!(status(&r, "database"), "skip");
    assert!(
        r["checks"].as_array().unwrap().len() > 8,
        "every check still has a line"
    );
}

#[tokio::test]
async fn a_database_that_is_down_fails() {
    let env = Env::at(
        "postgres://nobody:nothing@127.0.0.1:1/none".to_owned(),
        CONFIG,
    );

    let out = env
        .run(&[
            "doctor",
            "--json",
            "--set",
            "cli.core_url=http://127.0.0.1:1",
        ])
        .await;

    let r = report(&out);
    assert!(!out.ok);
    assert_eq!(status(&r, "database"), "fail");
    assert_eq!(status(&r, "schema"), "skip");
    assert_eq!(status(&r, "core"), "fail");
}

#[tokio::test]
async fn pending_migrations_fail() {
    bare_db_test(async |pool| {
        let env = Env::new(&pool);

        let out = env.run(&["doctor", "--json"]).await;

        let r = report(&out);
        assert!(!out.ok);
        assert_eq!(status(&r, "database"), "ok");
        assert_eq!(status(&r, "schema"), "fail");
        assert!(detail(&r, "schema").contains("pending"), "{r:#}");
    })
    .await;
}

#[tokio::test]
async fn a_schema_newer_than_the_binary_fails() {
    db_test(async |pool| {
        sqlx::query(
            "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
             VALUES (99999, 'from the future', true, '\\x00', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let env = Env::new(&pool);

        let out = env.run(&["doctor", "--json"]).await;

        let r = report(&out);
        assert!(!out.ok);
        assert_eq!(status(&r, "schema"), "fail");
        assert!(
            detail(&r, "schema").contains("newer than this binary"),
            "{r:#}"
        );
    })
    .await;
}

#[tokio::test]
async fn the_env_file_must_not_be_readable_by_others() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        let file = env.path(".env");
        std::fs::write(&file, format!("DATABASE_URL={}\n", env.url)).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

        // DATABASE_URL comes from the file alone.
        let open = env.run_with(&["doctor", "--json"], None, &[]).await;
        let r = report(&open);
        assert_eq!(status(&r, "environment"), "ok", "{r:#}");
        assert_eq!(status(&r, "env file"), "fail");
        assert!(detail(&r, "env file").contains("chmod 600"));
        assert!(!open.ok);

        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        let closed = env.run_with(&["doctor", "--json"], None, &[]).await;
        assert_eq!(status(&report(&closed), "env file"), "ok");
    })
    .await;
}

#[tokio::test]
async fn the_cli_and_the_server_must_be_one_release() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        let core = fake_core(json!({
            "status": "ok", "database": "ok", "version": "9.9.9", "modules": [],
        }))
        .await;
        let flag = format!("cli.core_url={core}");

        let out = env.run(&["doctor", "--json", "--set", &flag]).await;

        let r = report(&out);
        assert!(!out.ok);
        assert_eq!(status(&r, "versions"), "fail");
        assert!(detail(&r, "versions").contains("server 9.9.9"), "{r:#}");
    })
    .await;
}

#[tokio::test]
async fn the_cli_and_the_server_must_be_built_with_the_same_modules() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        let core = fake_core(json!({
            "status": "ok", "database": "ok", "version": env!("CARGO_PKG_VERSION"),
            "modules": ["some-other-module"],
        }))
        .await;
        let flag = format!("cli.core_url={core}");

        let out = env.run(&["doctor", "--json", "--set", &flag]).await;

        let r = report(&out);
        assert!(!out.ok);
        assert_eq!(status(&r, "versions"), "ok");
        assert_eq!(status(&r, "module sets"), "fail");
        assert!(
            detail(&r, "module sets").contains("VUMLO_FEATURES"),
            "{r:#}"
        );
    })
    .await;
}

#[tokio::test]
async fn an_unhealthy_core_fails() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        let core = fake_core(json!({
            "status": "ok", "database": "down", "version": "0.1.0", "modules": [],
        }))
        .await;
        let flag = format!("cli.core_url={core}");

        let out = env.run(&["doctor", "--json", "--set", &flag]).await;

        let r = report(&out);
        assert_eq!(status(&r, "core"), "fail");
        assert!(detail(&r, "core").contains("database=down"), "{r:#}");
    })
    .await;
}

#[tokio::test]
async fn the_web_build_must_be_newer_than_the_last_brand_apply() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        let generated = env.path(".vumlo/generated");
        std::fs::create_dir_all(&generated).unwrap();
        std::fs::write(generated.join("client-config.json"), "{}").unwrap();
        let build = env.path("web/.next");
        std::fs::create_dir_all(&build).unwrap();

        let missing = report(&env.run(&["doctor", "--json"]).await);
        assert_eq!(status(&missing, "web build"), "warn");
        assert!(detail(&missing, "web build").contains("not built"));

        let later =
            |seconds: u64| std::time::SystemTime::now() + std::time::Duration::from_secs(seconds);
        let touch = |path: std::path::PathBuf, at: std::time::SystemTime| {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(path)
                .unwrap();
            file.set_modified(at).unwrap();
        };
        touch(build.join("BUILD_ID"), later(60));
        let fresh = report(&env.run(&["doctor", "--json"]).await);
        assert_eq!(status(&fresh, "web build"), "ok");

        touch(generated.join("client-config.json"), later(120));
        let stale = report(&env.run(&["doctor", "--json"]).await);
        assert_eq!(status(&stale, "web build"), "warn");
        assert!(detail(&stale, "web build").contains("build web"));
    })
    .await;
}

#[tokio::test]
async fn the_human_report_has_a_line_per_check_and_a_verdict() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        let out = env
            .run(&["doctor", "--set", "cli.core_url=http://127.0.0.1:1"])
            .await;
        assert!(!out.ok);
        assert!(
            out.stdout.contains("[ ok ] configuration"),
            "{}",
            out.stdout
        );
        assert!(out.stdout.contains("[FAIL] core"), "{}", out.stdout);
        assert!(out.stdout.trim_end().ends_with("doctor: failed"));
    })
    .await;
}
