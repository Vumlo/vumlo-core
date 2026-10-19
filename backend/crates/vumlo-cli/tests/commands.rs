//! The commands of `vumlo-cli`, run as the real binary against throwaway databases.

mod common;

use common::{Env, ORIGIN, audit, audit_meta, bare_db_test, db_test, settings_for, spawn_server};
use serde_json::{Value, json};
use sqlx::PgPool;

const PASSWORD: &str = "correct horse battery staple";

async fn count(pool: &PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

// --- init -------------------------------------------------------------------------------------

#[tokio::test]
async fn init_writes_the_files_once_and_never_overwrites() {
    let dir = std::env::temp_dir().join(format!("vumlo-init-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let env = Env::at(String::new(), "mine = true\n");
    let _ = std::fs::remove_file(env.path("vumlo.toml"));
    let target = dir.to_str().unwrap();
    let at = |extra: &'static [&'static str]| {
        let mut all = vec!["init", "--dir", target];
        all.extend_from_slice(extra);
        all
    };

    let dry = env.run_with(&at(&["--dry-run"]), None, &[]).await;
    assert!(dry.ok, "{}", dry.stderr);
    assert!(dry.stdout.contains("would create  vumlo.toml"));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);

    let first = env.run_with(&at(&[]), None, &[]).await;
    assert!(
        first.ok && first.stdout.contains("created  vumlo.toml"),
        "{}",
        first.stderr
    );
    std::fs::write(dir.join("vumlo.toml"), "edited").unwrap();

    let second = env.run_with(&at(&[]), None, &[]).await;
    assert!(second.ok && second.stdout.contains("skipped  vumlo.toml"));
    assert_eq!(
        std::fs::read_to_string(dir.join("vumlo.toml")).unwrap(),
        "edited"
    );
    assert!(dir.join(".env.example").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

// --- config -----------------------------------------------------------------------------------

#[tokio::test]
async fn config_check_accepts_a_good_configuration() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        let out = env.run(&["config", "check"]).await;
        assert!(out.ok, "{}", out.stderr);
        assert!(out.stdout.contains("configuration OK"));
    })
    .await;
}

#[tokio::test]
async fn config_check_lists_every_problem_and_never_echoes_a_secret() {
    db_test(async |pool| {
        let config = format!(
            "{}\n[database]\nurl = \"postgres://file-user:file-secret@h/db\"\n[modules]\nnope = true\n",
            common::CONFIG
        );
        let env = Env::with_config(&pool, &config);

        let out = env.run(&["config", "check"]).await;

        assert!(!out.ok);
        assert!(out.stderr.contains("database.url is a secret"), "{}", out.stderr);
        assert!(out.stderr.contains("DATABASE_URL"), "{}", out.stderr);
        assert!(out.stderr.contains("modules.nope: unknown module"), "{}", out.stderr);
        assert!(!out.stderr.contains("file-secret") && !out.stdout.contains("file-secret"));
    })
    .await;
}

#[tokio::test]
async fn config_check_refuses_a_secret_set_by_a_flag() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        let out = env
            .run(&[
                "config",
                "check",
                "--set",
                "database.url=postgres://flag/db",
            ])
            .await;
        assert!(!out.ok);
        assert!(out.stderr.contains("--set"), "{}", out.stderr);
    })
    .await;
}

#[tokio::test]
async fn precedence_is_file_then_environment_then_flags() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        let url = env.url.as_str();
        let name = |out: &common::Out| -> Value {
            serde_json::from_str::<Value>(&out.stdout).unwrap()["institution"]["name"].clone()
        };

        let file = env.run(&["config", "show"]).await;
        assert_eq!(name(&file), "Test University");

        let from_env = env
            .run_with(
                &["config", "show"],
                None,
                &[
                    ("DATABASE_URL", url),
                    ("VUMLO__INSTITUTION__NAME", "From env"),
                ],
            )
            .await;
        assert_eq!(name(&from_env), "From env");

        let from_flag = env
            .run_with(
                &["config", "show", "--set", "institution.name=From flag"],
                None,
                &[
                    ("DATABASE_URL", url),
                    ("VUMLO__INSTITUTION__NAME", "From env"),
                ],
            )
            .await;
        assert_eq!(name(&from_flag), "From flag");
    })
    .await;
}

#[tokio::test]
async fn config_show_redacts_secrets_and_public_shows_only_public_keys() {
    db_test(async |pool| {
        let env = Env::new(&pool);

        let full = env.run(&["config", "show"]).await;
        assert!(full.ok, "{}", full.stderr);
        assert!(full.stdout.contains("<redacted>"));
        assert!(!full.stdout.contains(&env.url), "{}", full.stdout);

        let public = env.run(&["config", "show", "--public"]).await;
        assert!(public.ok);
        let value: Value = serde_json::from_str(&public.stdout).unwrap();
        assert_eq!(value["api_url"], format!("{ORIGIN}/api/v1"));
        assert_eq!(value["public"]["institution"]["name"], "Test University");
        assert!(value.get("database").is_none() && !public.stdout.contains("redacted"));
    })
    .await;
}

// --- modules, version -------------------------------------------------------------------------

#[tokio::test]
async fn modules_list_shows_the_core_modules_and_the_optional_ones() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        let out = env.run(&["modules", "list", "--json"]).await;
        assert!(out.ok, "{}", out.stderr);
        let value: Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(value["core"].as_array().unwrap().len(), 9);
        assert!(
            value["core"]
                .as_array()
                .unwrap()
                .contains(&json!("identity"))
        );
        #[cfg(not(feature = "dummy"))]
        assert_eq!(
            value["optional"],
            json!({}),
            "a default build has no optional modules"
        );
        #[cfg(feature = "dummy")]
        assert_eq!(value["optional"]["dummy"], "disabled");
    })
    .await;
}

#[tokio::test]
async fn an_unknown_module_name_is_refused() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        let out = env
            .run(&["config", "check", "--set", "modules.ai-agent=true"])
            .await;
        assert!(!out.ok);
        assert!(
            out.stderr.contains("modules.ai-agent: unknown module"),
            "{}",
            out.stderr
        );
    })
    .await;
}

#[tokio::test]
async fn version_reports_the_release_and_the_latest_migration() {
    let env = Env::at(String::new(), "");
    let out = env.run_with(&["version"], None, &[]).await;
    assert!(out.ok, "{}", out.stderr);
    assert!(
        out.stdout
            .starts_with(&format!("vumlo-cli {}", env!("CARGO_PKG_VERSION")))
    );
    assert!(out.stdout.contains("latest core migration"));
}

#[test]
fn the_web_app_has_the_version_of_the_release() {
    let package: Value =
        serde_json::from_str(include_str!("../../../../web/package.json")).unwrap();
    assert_eq!(package["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn the_desktop_app_has_the_version_of_the_release() {
    let package: Value =
        serde_json::from_str(include_str!("../../../../desktop/package.json")).unwrap();
    assert_eq!(package["version"], env!("CARGO_PKG_VERSION"));
    let cargo = include_str!("../../../../desktop/Cargo.toml");
    let line = format!("version = \"{}\"", env!("CARGO_PKG_VERSION"));
    assert!(
        cargo.lines().take(5).any(|l| l == line),
        "desktop/Cargo.toml says {line} in its [package]"
    );
}

#[test]
fn the_compose_example_names_its_images_after_the_release() {
    let compose = include_str!("../../../../deploy/compose.yaml");
    let tag = format!("VUMLO_VERSION:-{}}}", env!("CARGO_PKG_VERSION"));
    assert_eq!(compose.matches(&tag).count(), 2, "backend and web: {tag}");
}

// --- db ---------------------------------------------------------------------------------------

#[tokio::test]
async fn migrate_applies_everything_records_the_mode_and_audits_once() {
    bare_db_test(async |pool| {
        let env = Env::new(&pool);

        let dry = env.run(&["db", "migrate", "--dry-run"]).await;
        assert!(dry.ok, "{}", dry.stderr);
        assert!(dry.stdout.contains("would apply"));
        let tables = count(
            &pool,
            "SELECT count(*) FROM pg_tables WHERE tablename = '_sqlx_migrations'",
        )
        .await;
        assert_eq!(tables, 0, "a dry run changes nothing");

        let first = env.run(&["db", "migrate"]).await;
        assert!(first.ok, "{}", first.stderr);
        assert_eq!(
            audit(&pool).await,
            [("db.migrate".into(), "cli".into(), true, "database".into())]
        );
        let meta = audit_meta(&pool, "db.migrate").await;
        assert!(
            meta["applied"]["core"].as_array().unwrap().len() >= 28,
            "{meta}"
        );
        assert_eq!(
            count(&pool, "SELECT count(*) FROM org.installation").await,
            1
        );

        let second = env.run(&["db", "migrate"]).await;
        assert!(
            second.ok && second.stdout.contains("up to date"),
            "{}",
            second.stdout
        );
        assert_eq!(
            audit(&pool).await.len(),
            1,
            "a run that changed nothing writes no row"
        );
    })
    .await;
}

#[tokio::test]
async fn migrate_refuses_a_schema_newer_than_the_binary() {
    db_test(async |pool| {
        sqlx::query(
            "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
             VALUES (99999, 'from the future', true, '\\x00', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let env = Env::new(&pool);

        let migrate = env.run(&["db", "migrate"]).await;
        assert!(!migrate.ok);
        assert!(
            migrate.stderr.contains("newer than this binary"),
            "{}",
            migrate.stderr
        );

        let status = env.run(&["db", "status"]).await;
        assert!(!status.ok, "status exits non-zero for a newer schema");
        assert!(status.stdout.contains("newer release"), "{}", status.stdout);
        assert!(audit(&pool).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn migrate_refuses_a_changed_institution_mode() {
    bare_db_test(async |pool| {
        let env = Env::new(&pool);
        assert!(env.run(&["db", "migrate"]).await.ok);

        let other = env
            .run(&["db", "migrate", "--set", "institution.mode=school"])
            .await;

        assert!(!other.ok);
        assert!(
            other.stderr.contains("cannot be changed"),
            "{}",
            other.stderr
        );
    })
    .await;
}

// --- admin ------------------------------------------------------------------------------------

#[tokio::test]
async fn admin_create_makes_an_admin_who_can_log_in_and_audits_it() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        let out = env
            .run_with(
                &[
                    "admin",
                    "create",
                    "Root@School.edu",
                    "--name",
                    "Root",
                    "--surname",
                    "Admin",
                    "--password-stdin",
                ],
                Some(&format!("{PASSWORD}\n")),
                &[("DATABASE_URL", env.url.as_str())],
            )
            .await;
        assert!(out.ok, "{}", out.stderr);
        assert!(!out.stdout.contains(PASSWORD) && !out.stderr.contains(PASSWORD));

        let role: String =
            sqlx::query_scalar("SELECT role FROM identity.users WHERE email = 'root@school.edu'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(role, "admin");
        assert_eq!(
            audit(&pool).await,
            [("user.create".into(), "cli".into(), true, "user".into())]
        );
        assert_eq!(
            audit_meta(&pool, "user.create").await,
            json!({"role": "admin"})
        );

        // The real server accepts the password the CLI hashed.
        let base = spawn_server(&pool, settings_for(&env.url, common::CONFIG, &[])).await;
        let login = reqwest::Client::new()
            .post(format!("{base}/api/v1/auth/login"))
            .header("content-type", "application/json")
            .body(
                json!({"email": "root@school.edu", "password": PASSWORD, "client": "app"})
                    .to_string(),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(login.status(), 200);
    })
    .await;
}

#[tokio::test]
async fn admin_create_never_takes_the_password_as_an_argument() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        let out = env
            .run(&[
                "admin",
                "create",
                "a@school.edu",
                "--name",
                "A",
                "--surname",
                "B",
                "--password",
                PASSWORD,
            ])
            .await;
        assert!(!out.ok);
        assert!(out.stderr.contains("--password"), "{}", out.stderr);
        assert_eq!(count(&pool, "SELECT count(*) FROM identity.users").await, 0);
    })
    .await;
}

#[tokio::test]
async fn admin_create_rejects_a_weak_password_and_writes_nothing() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        let out = env
            .run_with(
                &[
                    "admin",
                    "create",
                    "a@school.edu",
                    "--name",
                    "A",
                    "--surname",
                    "B",
                    "--password-stdin",
                ],
                Some("password1234\n"),
                &[("DATABASE_URL", env.url.as_str())],
            )
            .await;
        assert!(!out.ok);
        assert!(out.stderr.contains("password rejected"), "{}", out.stderr);
        assert_eq!(count(&pool, "SELECT count(*) FROM identity.users").await, 0);
        assert!(audit(&pool).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn admin_create_is_idempotent_and_has_a_dry_run() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        let create = |extra: &'static [&'static str]| {
            let mut args = vec![
                "admin",
                "create",
                "root@school.edu",
                "--name",
                "Root",
                "--surname",
                "Admin",
                "--password-stdin",
            ];
            args.extend_from_slice(extra);
            args
        };
        let stdin = format!("{PASSWORD}\n");
        let vars = [("DATABASE_URL", env.url.as_str())];

        let dry = env
            .run_with(&create(&["--dry-run"]), Some(&stdin), &vars)
            .await;
        assert!(
            dry.ok && dry.stdout.contains("would create"),
            "{}",
            dry.stderr
        );
        assert_eq!(count(&pool, "SELECT count(*) FROM identity.users").await, 0);

        assert!(env.run_with(&create(&[]), Some(&stdin), &vars).await.ok);
        let again = env.run_with(&create(&[]), Some(&stdin), &vars).await;
        assert!(
            again.ok && again.stdout.contains("already exists"),
            "{}",
            again.stdout
        );
        assert_eq!(count(&pool, "SELECT count(*) FROM identity.users").await, 1);
        assert_eq!(audit(&pool).await.len(), 1);
    })
    .await;
}

#[tokio::test]
async fn admin_create_refuses_an_unmigrated_database() {
    bare_db_test(async |pool| {
        let env = Env::new(&pool);
        let out = env
            .run_with(
                &[
                    "admin",
                    "create",
                    "a@school.edu",
                    "--name",
                    "A",
                    "--surname",
                    "B",
                    "--password-stdin",
                ],
                Some(&format!("{PASSWORD}\n")),
                &[("DATABASE_URL", env.url.as_str())],
            )
            .await;
        assert!(!out.ok);
        assert!(out.stderr.contains("not ready"), "{}", out.stderr);
    })
    .await;
}
