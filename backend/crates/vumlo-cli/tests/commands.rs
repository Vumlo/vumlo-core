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
        assert!(
            out.stderr
                .contains("modules.nope: unknown, or not compiled into this binary"),
            "{}",
            out.stderr
        );
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

#[tokio::test]
async fn config_schema_prints_the_json_schema_of_the_configuration() {
    db_test(async |pool| {
        let env = Env::new(&pool);

        let out = env.run(&["config", "schema"]).await;

        assert!(out.ok, "{}", out.stderr);
        let schema: Value = serde_json::from_str(&out.stdout).unwrap();
        assert!(
            schema["properties"]["institution"].is_object()
                || schema["$defs"]["InstitutionSection"].is_object(),
            "{}",
            out.stdout
        );
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
        assert!(value["core"].as_array().unwrap().contains(&json!("auth")));
        // Exactly the modules this build contains, each off until the configuration turns it on;
        // a default build has none.
        let compiled: &[&str] = &[
            #[cfg(feature = "dummy")]
            "dummy",
            #[cfg(feature = "ai_agent")]
            "ai_agent",
        ];
        let optional = value["optional"].as_object().unwrap();
        assert_eq!(optional.len(), compiled.len(), "{optional:?}");
        for name in compiled {
            assert_eq!(optional[*name], "disabled", "{name}");
        }
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
            out.stderr
                .contains("modules.ai-agent: unknown, or not compiled into this binary"),
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
fn the_admin_console_has_the_version_of_the_release() {
    let package: Value =
        serde_json::from_str(include_str!("../../../../web-admin/package.json")).unwrap();
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
    assert_eq!(compose.matches(&tag).count(), 3, "backend, web and console: {tag}");
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
            sqlx::query_scalar("SELECT role FROM auth.users WHERE email = 'root@school.edu'")
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
        let base = spawn_server(&pool, settings_for(&env.url, common::CONFIG)).await;
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
        assert_eq!(count(&pool, "SELECT count(*) FROM auth.users").await, 0);
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
        assert_eq!(count(&pool, "SELECT count(*) FROM auth.users").await, 0);
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
        assert_eq!(count(&pool, "SELECT count(*) FROM auth.users").await, 0);

        assert!(env.run_with(&create(&[]), Some(&stdin), &vars).await.ok);
        let again = env.run_with(&create(&[]), Some(&stdin), &vars).await;
        assert!(
            again.ok && again.stdout.contains("already exists"),
            "{}",
            again.stdout
        );
        assert_eq!(count(&pool, "SELECT count(*) FROM auth.users").await, 1);
        assert_eq!(audit(&pool).await.len(), 1);
    })
    .await;
}

const NEW_PASSWORD: &str = "another long passphrase 7";

/// Creates the admin `root@school.edu` with [`PASSWORD`] through the CLI, as an installation does.
async fn create_root(env: &Env) {
    let out = env
        .run_with(
            &[
                "admin",
                "create",
                "root@school.edu",
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
}

async fn set_password(env: &Env, email: &str, stdin: &str, extra: &[&str]) -> common::Out {
    let mut args = vec!["admin", "set-password", email, "--password-stdin"];
    args.extend_from_slice(extra);
    env.run_with(
        &args,
        Some(&format!("{stdin}\n")),
        &[("DATABASE_URL", env.url.as_str())],
    )
    .await
}

async fn console_status(base: &str, email: &str, password: &str) -> u16 {
    reqwest::Client::new()
        .post(format!("{base}/api/v1/console/login"))
        .header("content-type", "application/json")
        .header("origin", "https://admin.lms.test")
        .body(json!({"email": email, "password": password}).to_string())
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test]
async fn set_password_replaces_the_password_and_signs_the_admin_out() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        create_root(&env).await;
        let base = spawn_server(&pool, settings_for(&env.url, common::CONFIG)).await;
        assert_eq!(
            console_status(&base, "root@school.edu", PASSWORD).await,
            200
        );
        let app = reqwest::Client::new()
            .post(format!("{base}/api/v1/auth/login"))
            .header("content-type", "application/json")
            .body(
                json!({"email": "root@school.edu", "password": PASSWORD, "client": "app"})
                    .to_string(),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(app.status(), 200);
        assert_eq!(count(&pool, "SELECT count(*) FROM auth.sessions").await, 2);

        let out = set_password(&env, "Root@School.edu", NEW_PASSWORD, &[]).await;
        assert!(out.ok, "{}", out.stderr);
        assert!(!out.stdout.contains(NEW_PASSWORD) && !out.stderr.contains(NEW_PASSWORD));
        assert_eq!(count(&pool, "SELECT count(*) FROM auth.sessions").await, 0);
        assert_eq!(
            console_status(&base, "root@school.edu", PASSWORD).await,
            401
        );
        assert_eq!(
            console_status(&base, "root@school.edu", NEW_PASSWORD).await,
            200
        );
        // The admin chose this password for themselves: nothing makes them change it at once.
        let must_change: bool = sqlx::query_scalar(
            "SELECT must_change_password FROM auth.users WHERE email = 'root@school.edu'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!must_change);
    })
    .await;
}

#[tokio::test]
async fn set_password_audits_with_source_cli_and_no_secret() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        create_root(&env).await;
        let out = set_password(&env, "root@school.edu", NEW_PASSWORD, &[]).await;
        assert!(out.ok, "{}", out.stderr);
        assert_eq!(
            audit(&pool).await,
            [
                ("user.create".into(), "cli".into(), true, "user".into()),
                (
                    "user.password_set".into(),
                    "cli".into(),
                    true,
                    "user".into()
                )
            ]
        );
        assert_eq!(audit_meta(&pool, "user.password_set").await, json!({}));
    })
    .await;
}

#[tokio::test]
async fn set_password_refuses_a_teacher_and_an_unknown_email_the_same_way() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        create_root(&env).await;
        sqlx::query(
            "INSERT INTO auth.users (id, email, name, surname, role, email_verified_at)
             VALUES ($1, 'teacher@school.edu', 'T', 'T', 'teacher', now())",
        )
        .bind(uuid::Uuid::now_v7())
        .execute(&pool)
        .await
        .unwrap();
        let teacher = set_password(&env, "teacher@school.edu", NEW_PASSWORD, &[]).await;
        let unknown = set_password(&env, "nobody@school.edu", NEW_PASSWORD, &[]).await;
        assert!(!teacher.ok && !unknown.ok);
        assert!(
            teacher.stderr.contains("no active administrator"),
            "{}",
            teacher.stderr
        );
        assert_eq!(
            teacher.stderr.replace("teacher@school.edu", "X"),
            unknown.stderr.replace("nobody@school.edu", "X")
        );
        assert_eq!(audit(&pool).await.len(), 1, "only the creation is audited");
        let hashes = count(
            &pool,
            "SELECT count(DISTINCT password_hash) FROM auth.identities",
        )
        .await;
        assert_eq!(hashes, 1, "nobody's password changed");
    })
    .await;
}

#[tokio::test]
async fn set_password_has_a_dry_run_and_rejects_a_weak_password() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        create_root(&env).await;
        let before: String = sqlx::query_scalar("SELECT password_hash FROM auth.identities")
            .fetch_one(&pool)
            .await
            .unwrap();
        let dry = set_password(&env, "root@school.edu", NEW_PASSWORD, &["--dry-run"]).await;
        assert!(dry.ok && dry.stdout.contains("would set"), "{}", dry.stderr);
        let weak = set_password(&env, "root@school.edu", "password1234", &[]).await;
        assert!(!weak.ok);
        assert!(weak.stderr.contains("password rejected"), "{}", weak.stderr);
        let after: String = sqlx::query_scalar("SELECT password_hash FROM auth.identities")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(before, after);
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
