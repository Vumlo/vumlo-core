//! `vumlo-cli plugin ...` (ADR-0066): the administrator approves scopes, the token and the secret are
//! shown once, every change is audited with the source `cli`, and a command that changes nothing
//! says so and writes nothing.

mod common;

use axum::routing::get;
use axum::{Json, Router};
use common::{Env, Out, audit, audit_meta, db_test};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::net::TcpListener;

const KEY: &str = "0123456789abcdef0123456789abcdef";

/// A plugin that serves only its manifest, asking for `scopes`. Returns where it listens.
async fn plugin(scopes: &[&str]) -> String {
    let manifest = json!({
        "id": "hello", "version": "1.0.0", "api": "1",
        "subscribes": ["submission.submitted"], "scopes": scopes
    });
    let app = Router::new().route(
        "/manifest",
        get(move || {
            let manifest = manifest.clone();
            async move { Json(manifest) }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });
    base
}

async fn run(env: &Env, args: &[&str], stdin: Option<&str>) -> Out {
    let vars = [
        ("DATABASE_URL", env.url.as_str()),
        ("VUMLO__PLUGINS__SECRET_KEY", KEY),
    ];
    env.run_with(args, stdin, &vars).await
}

fn value_of(out: &Out, name: &str) -> String {
    out.stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no {name} in:\n{}", out.stdout))
        .to_owned()
}

async fn scopes_of_hello(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT scopes FROM plugins.plugin_installs WHERE plugin_id = 'hello'")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn installs(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM plugins.plugin_installs")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn install_shows_the_token_and_the_secret_once_and_stores_neither() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        let base = plugin(&["submissions:read", "reports:write"]).await;

        let out = run(
            &env,
            &["plugin", "install", &base, "--grant", "submissions:read"],
            None,
        )
        .await;

        assert!(out.ok, "{}", out.stderr);
        let token = value_of(&out, "VUMLO_PLUGIN_TOKEN");
        let secret = value_of(&out, "VUMLO_WEBHOOK_SECRET");
        assert!(token.starts_with("vplg_") && !secret.is_empty());
        assert_eq!(scopes_of_hello(&pool).await, ["submissions:read"]);
        let row: String =
            sqlx::query_scalar("SELECT row_to_json(t)::text FROM plugins.plugin_installs t")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!row.contains(&token) && !row.contains(&secret));

        // One audit row, with no actor, and nothing secret in it.
        let rows = audit(&pool).await;
        assert_eq!(
            rows,
            [(
                "plugin.install".to_owned(),
                "cli".to_owned(),
                true,
                "plugin".to_owned()
            )]
        );
        let meta = audit_meta(&pool, "plugin.install").await;
        assert_eq!(meta["plugin_id"], "hello");
        assert_eq!(meta["scopes"], json!(["submissions:read"]));
        assert!(!meta.to_string().contains(&token) && !meta.to_string().contains(&secret));

        // `list` says what was asked for and what was granted, and shows no credential.
        let listed = run(&env, &["plugin", "list"], None).await;
        assert!(listed.ok, "{}", listed.stderr);
        assert!(listed.stdout.contains("hello"), "{}", listed.stdout);
        assert!(
            listed
                .stdout
                .contains("scopes: submissions:read (asked for: submissions:read, reports:write)")
        );
        assert!(!listed.stdout.contains(&token) && !listed.stdout.contains(&secret));
    })
    .await;
}

#[tokio::test]
async fn a_scope_nobody_asked_for_or_that_does_not_exist_stops_the_install() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        let base = plugin(&["submissions:read"]).await;

        let not_asked = run(
            &env,
            &["plugin", "install", &base, "--grant", "reports:write"],
            None,
        )
        .await;
        assert!(!not_asked.ok);
        assert!(
            not_asked
                .stderr
                .contains("`reports:write` cannot be granted"),
            "{}",
            not_asked.stderr
        );

        let unknown = run(
            &env,
            &["plugin", "install", &base, "--grant", "users:read"],
            None,
        )
        .await;
        assert!(!unknown.ok);
        assert!(
            unknown.stderr.contains("unknown scope `users:read`"),
            "{}",
            unknown.stderr
        );

        assert_eq!(installs(&pool).await, 0);
        assert!(audit(&pool).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn without_a_grant_flag_each_requested_scope_is_asked_for() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        let base = plugin(&["submissions:read", "reports:write"]).await;

        let out = run(&env, &["plugin", "install", &base], Some("y\nn\n")).await;

        assert!(out.ok, "{}", out.stderr);
        assert!(
            out.stdout.contains("grant `submissions:read`?")
                && out.stdout.contains("grant `reports:write`?")
        );
        assert_eq!(scopes_of_hello(&pool).await, ["submissions:read"]);
    })
    .await;
}

#[tokio::test]
async fn a_plugin_may_be_installed_with_nothing_granted() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        let base = plugin(&["submissions:read"]).await;

        let out = run(&env, &["plugin", "install", &base, "--grant", "none"], None).await;

        assert!(out.ok, "{}", out.stderr);
        assert!(scopes_of_hello(&pool).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn an_install_needs_the_key_and_a_safe_address_and_an_id_is_installed_once() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        let base = plugin(&["submissions:read"]).await;

        let no_key = env
            .run_with(
                &["plugin", "install", &base, "--grant", "none"],
                None,
                &[("DATABASE_URL", env.url.as_str())],
            )
            .await;
        assert!(!no_key.ok);
        assert!(
            no_key.stderr.contains("VUMLO__PLUGINS__SECRET_KEY"),
            "{}",
            no_key.stderr
        );

        let plain = run(
            &env,
            &[
                "plugin",
                "install",
                "http://tools.example.edu",
                "--grant",
                "none",
            ],
            None,
        )
        .await;
        assert!(!plain.ok);
        assert!(plain.stderr.contains("https"), "{}", plain.stderr);

        let key_in_a_flag = run(
            &env,
            &["--set", "plugins.secret_key=abc", "plugin", "list"],
            None,
        )
        .await;
        assert!(!key_in_a_flag.ok);
        assert!(
            key_in_a_flag.stderr.contains("secret"),
            "{}",
            key_in_a_flag.stderr
        );

        assert!(
            run(&env, &["plugin", "install", &base, "--grant", "none"], None)
                .await
                .ok
        );
        let again = run(&env, &["plugin", "install", &base, "--grant", "none"], None).await;
        assert!(!again.ok);
        assert!(
            again.stderr.contains("installed already"),
            "{}",
            again.stderr
        );
        assert_eq!(installs(&pool).await, 1);
    })
    .await;
}

#[tokio::test]
async fn every_change_is_audited_and_a_command_that_changes_nothing_says_so() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        let base = plugin(&["submissions:read", "reports:write"]).await;
        let installed = run(
            &env,
            &[
                "plugin",
                "install",
                &base,
                "--grant",
                "submissions:read,reports:write",
            ],
            None,
        )
        .await;
        assert!(installed.ok, "{}", installed.stderr);
        let token = value_of(&installed, "VUMLO_PLUGIN_TOKEN");
        let audited = async || audit(&pool).await.len();
        assert_eq!(audited().await, 1);

        // Grant: replaces the scopes, audited with before and after.
        let grant = run(&env, &["plugin", "grant", "hello", "reports:write"], None).await;
        assert!(grant.ok, "{}", grant.stderr);
        assert_eq!(scopes_of_hello(&pool).await, ["reports:write"]);
        let meta = audit_meta(&pool, "plugin.grant").await;
        assert_eq!(meta["from"], json!(["submissions:read", "reports:write"]));
        assert_eq!(meta["to"], json!(["reports:write"]));
        assert_eq!(audited().await, 2);
        let same = run(&env, &["plugin", "grant", "hello", "reports:write"], None).await;
        assert!(
            same.ok && same.stdout.contains("nothing was changed"),
            "{}",
            same.stdout
        );
        let beyond = run(&env, &["plugin", "grant", "hello", "users:read"], None).await;
        assert!(!beyond.ok);
        assert_eq!(audited().await, 2);

        // Disable and enable: audited, idempotent, and a dry run changes nothing.
        let dry = run(&env, &["plugin", "disable", "hello", "--dry-run"], None).await;
        assert!(
            dry.ok && dry.stdout.contains("would make hello disabled"),
            "{}",
            dry.stdout
        );
        assert_eq!(audited().await, 2);
        assert!(run(&env, &["plugin", "disable", "hello"], None).await.ok);
        let status: String = sqlx::query_scalar("SELECT status FROM plugins.plugin_installs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(status, "disabled");
        let again = run(&env, &["plugin", "disable", "hello"], None).await;
        assert!(
            again.ok && again.stdout.contains("disabled already"),
            "{}",
            again.stdout
        );
        assert_eq!(audited().await, 3);
        assert!(run(&env, &["plugin", "enable", "hello"], None).await.ok);
        assert_eq!(audited().await, 4);

        // Remove: refuses without `--yes`, looks with `--dry-run`, deletes with `--yes`.
        let refused = run(&env, &["plugin", "remove", "hello"], None).await;
        assert!(
            !refused.ok && refused.stderr.contains("--yes"),
            "{}",
            refused.stderr
        );
        assert!(
            run(&env, &["plugin", "remove", "hello", "--dry-run"], None)
                .await
                .ok
        );
        assert_eq!(installs(&pool).await, 1);
        let removed = run(&env, &["plugin", "remove", "hello", "--yes"], None).await;
        assert!(removed.ok, "{}", removed.stderr);
        assert_eq!(installs(&pool).await, 0);
        let rows = audit(&pool).await;
        assert_eq!(
            rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
            [
                "plugin.install",
                "plugin.grant",
                "plugin.disable",
                "plugin.enable",
                "plugin.remove"
            ]
        );
        assert!(rows.iter().all(|r| r.1 == "cli" && r.2));
        // No credential reached the audit log.
        let everything: Value = sqlx::query_scalar("SELECT jsonb_agg(meta) FROM events.audit_log")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(!everything.to_string().contains(&token));

        // A plugin that is not there is an error that names it.
        let missing = run(&env, &["plugin", "disable", "hello"], None).await;
        assert!(
            !missing.ok && missing.stderr.contains("there is no plugin `hello`"),
            "{}",
            missing.stderr
        );
    })
    .await;
}
