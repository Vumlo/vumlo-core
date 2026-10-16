//! Runs the real `learnly-server` binary against a test database.

mod common;

use std::process::Command;

use common::{Http, PASSWORD, login_app, spawn};
use serde_json::json;
use sqlx::PgPool;

struct Output {
    ok: bool,
    stdout: String,
    stderr: String,
}

/// `DATABASE_URL` from the environment, or from the workspace `.env` as `sqlx::test` does.
fn database_url() -> String {
    if let Ok(url) = std::env::var("DATABASE_URL") {
        return url;
    }
    let env_file = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.env"))
        .expect("DATABASE_URL is not set and there is no .env");
    env_file
        .lines()
        .find_map(|line| line.strip_prefix("DATABASE_URL="))
        .expect("DATABASE_URL in .env")
        .trim()
        .to_owned()
}

/// The URL of the throwaway database `#[sqlx::test]` created for this test.
fn test_database_url(pool: &PgPool) -> String {
    let url = database_url();
    let server = url
        .split('?')
        .next()
        .and_then(|u| u.rsplit_once('/'))
        .expect("url")
        .0;
    let database = pool
        .connect_options()
        .get_database()
        .expect("database")
        .to_owned();
    format!("{server}/{database}")
}

async fn run(pool: &PgPool, args: &[&str]) -> Output {
    run_in_mode(pool, "university", args).await
}

async fn run_in_mode(pool: &PgPool, mode: &str, args: &[&str]) -> Output {
    let mode = mode.to_owned();
    let url = test_database_url(pool);
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    tokio::task::spawn_blocking(move || {
        let output = Command::new(env!("CARGO_BIN_EXE_learnly-server"))
            .args(&args)
            .env("LEARNLY__SERVER__BIND_ADDR", "127.0.0.1:0")
            .env("LEARNLY__SERVER__PUBLIC_ORIGIN", common::ORIGIN)
            .env("LEARNLY__DATABASE__URL", url)
            .env("LEARNLY__DATABASE__MAX_CONNECTIONS", "2")
            .env("LEARNLY__LOG__FORMAT", "json")
            .env("LEARNLY__INSTITUTION__NAME", "Test University")
            .env("LEARNLY__INSTITUTION__MODE", &mode)
            .output()
            .expect("run learnly-server");
        Output {
            ok: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    })
    .await
    .expect("join")
}

fn temporary_password(output: &Output) -> String {
    output
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix("Temporary password: "))
        .unwrap_or_else(|| panic!("no password in {:?}", output.stdout))
        .to_owned()
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn create_admin_creates_a_working_admin_who_must_change_password(pool: PgPool) {
    let output = run(
        &pool,
        &[
            "create-admin",
            "Root@School.edu",
            "--name",
            "Root",
            "--surname",
            "Admin",
        ],
    )
    .await;

    assert!(output.ok, "{}", output.stderr);
    let temporary = temporary_password(&output);
    assert_eq!(temporary.len(), 22);
    assert!(
        !output.stdout.contains("root@school.edu"),
        "emails are not echoed"
    );

    let http = Http::new(&spawn(pool.clone()).await);
    let token = login_app(&http, "root@school.edu", &temporary).await;
    let me = http.get("/me").bearer(&token).send().await.json();
    assert_eq!(me["role"], "admin");
    assert_eq!(me["name"], "Root");
    assert_eq!(me["surname"], "Admin");
    assert_eq!(me["must_change_password"], true);

    // Until the password is changed, nothing but /me, logout and change-password works.
    http.get("/admin/users")
        .bearer(&token)
        .send()
        .await
        .problem(403);
    let changed = http
        .post("/me/password")
        .bearer(&token)
        .json(&json!({"current_password": temporary, "new_password": PASSWORD}))
        .send()
        .await;
    assert_eq!(changed.status, 204);
    assert_eq!(
        http.get("/admin/users").bearer(&token).send().await.status,
        200
    );
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn create_user_takes_a_role(pool: PgPool) {
    let output = run(
        &pool,
        &[
            "create-user",
            "terry@school.edu",
            "--name",
            "Terry",
            "--surname",
            "Teacher",
            "--role",
            "teacher",
        ],
    )
    .await;

    assert!(output.ok, "{}", output.stderr);
    let http = Http::new(&spawn(pool.clone()).await);
    let token = login_app(&http, "terry@school.edu", &temporary_password(&output)).await;
    assert_eq!(
        http.get("/me").bearer(&token).send().await.json()["role"],
        "teacher"
    );
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn create_user_refuses_bad_input_and_duplicates(pool: PgPool) {
    let created = run(
        &pool,
        &[
            "create-admin",
            "root@school.edu",
            "--name",
            "Root",
            "--surname",
            "Admin",
        ],
    )
    .await;
    assert!(created.ok, "{}", created.stderr);

    for (args, expected) in [
        (
            vec![
                "create-admin",
                "ROOT@school.edu",
                "--name",
                "Again",
                "--surname",
                "Admin",
            ],
            "already in use",
        ),
        (
            vec![
                "create-admin",
                "not-an-email",
                "--name",
                "X",
                "--surname",
                "Admin",
            ],
            "invalid email",
        ),
        (
            vec![
                "create-admin",
                "x@school.edu",
                "--name",
                "  ",
                "--surname",
                "X",
            ],
            "invalid name",
        ),
        (
            vec![
                "create-admin",
                "x@school.edu",
                "--name",
                "X",
                "--surname",
                "  ",
            ],
            "invalid surname",
        ),
        (
            vec![
                "create-user",
                "x@school.edu",
                "--name",
                "X",
                "--surname",
                "X",
                "--role",
                "owner",
            ],
            "teacher",
        ),
    ] {
        let output = run(&pool, &args).await;
        assert!(!output.ok, "{args:?} should fail");
        assert!(
            output.stderr.contains(expected),
            "{args:?}: {}",
            output.stderr
        );
        assert!(!output.stdout.contains("Temporary password"), "{args:?}");
    }
    let users = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM identity.users"#)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(users, 1);
}

#[sqlx::test(migrations = false)]
async fn create_admin_refuses_an_unmigrated_database(pool: PgPool) {
    let output = run(
        &pool,
        &[
            "create-admin",
            "root@school.edu",
            "--name",
            "Root",
            "--surname",
            "Admin",
        ],
    )
    .await;

    assert!(!output.ok);
    assert!(
        output.stderr.contains("learnly-server migrate"),
        "{}",
        output.stderr
    );
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn the_first_command_records_the_mode_and_a_different_mode_is_refused(pool: PgPool) {
    let admin = [
        "create-admin",
        "root@school.edu",
        "--name",
        "Root",
        "--surname",
        "Admin",
    ];
    assert!(run_in_mode(&pool, "university", &admin).await.ok);
    let recorded = sqlx::query_scalar!("SELECT mode FROM org.installation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(recorded, "university");

    let teacher = [
        "create-user",
        "t@school.edu",
        "--name",
        "T",
        "--surname",
        "Teacher",
        "--role",
        "teacher",
    ];
    let refused = run_in_mode(&pool, "school", &teacher).await;
    assert!(!refused.ok);
    for needle in ["institution.mode", "school", "university"] {
        assert!(
            refused.stderr.contains(needle),
            "{needle}: {}",
            refused.stderr
        );
    }
    let users = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM identity.users"#)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(users, 1);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_missing_mode_stops_the_binary_at_startup(pool: PgPool) {
    let url = test_database_url(&pool);
    let output = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_learnly-server"))
            .arg("serve")
            .env_remove("LEARNLY__INSTITUTION__MODE")
            .env("LEARNLY__SERVER__BIND_ADDR", "127.0.0.1:0")
            .env("LEARNLY__SERVER__PUBLIC_ORIGIN", common::ORIGIN)
            .env("LEARNLY__DATABASE__URL", url)
            .env("LEARNLY__DATABASE__MAX_CONNECTIONS", "2")
            .env("LEARNLY__LOG__FORMAT", "json")
            .env("LEARNLY__INSTITUTION__NAME", "Test University")
            .current_dir(env!("CARGO_TARGET_TMPDIR"))
            .output()
            .expect("run learnly-server")
    })
    .await
    .expect("join");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("institution.mode"), "{stderr}");
}
