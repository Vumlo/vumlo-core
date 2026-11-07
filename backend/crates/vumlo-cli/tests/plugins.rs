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
    serve(json!({
        "id": "hello", "version": "1.0.0", "api": "1",
        "subscribes": ["submission.submitted"], "scopes": scopes
    }))
    .await
}

/// A plugin that serves only this manifest. Returns where it listens.
async fn serve(manifest: Value) -> String {
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
            &[
                "plugin",
                "install",
                &base,
                "--grant",
                "submissions:read",
                "--courses",
                "all",
            ],
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
            &[
                "plugin",
                "install",
                &base,
                "--grant",
                "reports:write",
                "--courses",
                "all",
            ],
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
            &[
                "plugin",
                "install",
                &base,
                "--grant",
                "users:read",
                "--courses",
                "all",
            ],
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

        let out = run(&env, &["plugin", "install", &base], Some("y\nn\nall\n")).await;

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

        let out = run(
            &env,
            &[
                "plugin",
                "install",
                &base,
                "--grant",
                "none",
                "--courses",
                "all",
            ],
            None,
        )
        .await;

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
                &[
                    "plugin",
                    "install",
                    &base,
                    "--grant",
                    "none",
                    "--courses",
                    "all",
                ],
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
                "--courses",
                "all",
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
            run(
                &env,
                &[
                    "plugin",
                    "install",
                    &base,
                    "--grant",
                    "none",
                    "--courses",
                    "all"
                ],
                None
            )
            .await
            .ok
        );
        let again = run(
            &env,
            &[
                "plugin",
                "install",
                &base,
                "--grant",
                "none",
                "--courses",
                "all",
            ],
            None,
        )
        .await;
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
                "--courses",
                "all",
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

// ---------------------------------------------------------------------------------------------
// Settings (ADR-0072)
// ---------------------------------------------------------------------------------------------

async fn configurable() -> String {
    serve(json!({
        "id": "hello", "version": "1.0.0", "api": "1",
        "subscribes": ["plugin.settings.changed"], "scopes": ["settings:read"],
        "settings": {
            "unit": {"type": "string", "enum": ["words", "characters"], "default": "words",
                     "description": "What to count"},
            "api_token": {"type": "string", "secret": true},
            "limit": {"type": "number", "minimum": 1},
            "tags": {"type": "array", "items": {"type": "string"}},
            "max_words": {"type": "number", "scope": "course"}
        }
    }))
    .await
}

async fn install_configurable(env: &Env) {
    let base = configurable().await;
    let out = run(
        env,
        &[
            "plugin",
            "install",
            &base,
            "--grant",
            "settings:read",
            "--courses",
            "all",
        ],
        None,
    )
    .await;
    assert!(out.ok, "{}", out.stderr);
    assert!(
        out.stdout
            .contains("declares settings: api_token, limit, max_words, tags, unit"),
        "{}",
        out.stdout
    );
}

async fn setting_rows(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM plugins.settings")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn settings_are_listed_set_read_and_unset_with_one_audit_row_each_and_no_value_in_it() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        install_configurable(&env).await;

        let listed = run(&env, &["plugin", "settings", "list", "hello"], None).await;
        assert!(listed.ok, "{}", listed.stderr);
        for line in [
            "unit",
            "words (default)",
            "What to count",
            "api_token",
            "(not set)",
            "per course: add --course <id>",
        ] {
            assert!(
                listed.stdout.contains(line),
                "{line} in:\n{}",
                listed.stdout
            );
        }

        // Text stays text; a number, a boolean and a list are JSON.
        for (key, value) in [
            ("unit", "characters"),
            ("limit", "12"),
            ("tags", r#"["a","b"]"#),
        ] {
            let out = run(
                &env,
                &["plugin", "settings", "set", "hello", key, value],
                None,
            )
            .await;
            assert!(out.ok, "{key}: {}", out.stderr);
        }
        let get = |key: &'static str| {
            let env = &env;
            async move {
                run(env, &["plugin", "settings", "get", "hello", key], None)
                    .await
                    .stdout
                    .trim()
                    .to_owned()
            }
        };
        assert_eq!(get("unit").await, "characters");
        assert_eq!(get("limit").await, "12");
        assert_eq!(get("tags").await, r#"["a","b"]"#);

        // The same value again changes nothing and writes nothing.
        let again = run(
            &env,
            &["plugin", "settings", "set", "hello", "unit", "characters"],
            None,
        )
        .await;
        assert!(
            again.ok && again.stdout.contains("nothing was changed"),
            "{}",
            again.stdout
        );

        let unset = run(
            &env,
            &["plugin", "settings", "unset", "hello", "unit"],
            None,
        )
        .await;
        assert!(unset.ok, "{}", unset.stderr);
        assert_eq!(get("unit").await, "words (default)");
        let again = run(
            &env,
            &["plugin", "settings", "unset", "hello", "unit"],
            None,
        )
        .await;
        assert!(again.stdout.contains("nothing was changed"));

        // Audit: the install, then three sets and one unset; never a value.
        let rows = audit(&pool).await;
        let actions: Vec<_> = rows.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(
            actions,
            [
                "plugin.install",
                "plugin.setting.set",
                "plugin.setting.set",
                "plugin.setting.set",
                "plugin.setting.unset"
            ]
        );
        let meta = audit_meta(&pool, "plugin.setting.unset").await;
        assert_eq!(
            meta,
            json!({"plugin_id": "hello", "key": "unit", "course_id": null})
        );
        let event_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events.outbox WHERE type = 'plugin.settings.changed'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(event_count, 4, "one per change, none for the repeats");
    })
    .await;
}

#[tokio::test]
async fn a_secret_is_read_from_standard_input_stored_sealed_and_never_printed() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        install_configurable(&env).await;

        let out = run(
            &env,
            &["plugin", "settings", "set", "hello", "api_token", "-"],
            Some("tok-s3cret\n"),
        )
        .await;
        assert!(out.ok, "{}", out.stderr);
        assert!(!out.stdout.contains("s3cret") && !out.stderr.contains("s3cret"));

        let stored: String =
            sqlx::query_scalar("SELECT value::text FROM plugins.settings WHERE key = 'api_token'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!stored.contains("s3cret"));
        for args in [
            vec!["plugin", "settings", "get", "hello", "api_token"],
            vec!["plugin", "settings", "list", "hello"],
        ] {
            let shown = run(&env, &args, None).await;
            assert!(
                shown.ok && shown.stdout.contains("(set)") && !shown.stdout.contains("s3cret"),
                "{}",
                shown.stdout
            );
        }
        let meta = audit_meta(&pool, "plugin.setting.set").await;
        assert!(!meta.to_string().contains("s3cret"));
    })
    .await;
}

#[tokio::test]
async fn a_value_the_spec_refuses_a_wrong_scope_and_an_unknown_course_change_nothing() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        install_configurable(&env).await;
        let course = uuid::Uuid::now_v7().to_string();

        for (args, message) in [
            (
                vec!["unit", "furlongs"],
                "must be one of: words, characters",
            ),
            (vec!["limit", "0"], "must be at least 1"),
            (vec!["limit", "many"], "must be a number"),
            (vec!["tags", "a,b"], "must be a list"),
            (vec!["nope", "1"], "has no setting `nope`"),
            (vec!["max_words", "5"], "give the course"),
            (
                vec!["unit", "words", "--course", &course],
                "belongs to no course",
            ),
            (
                vec!["max_words", "5", "--course", &course],
                "no such course",
            ),
        ] {
            let mut full = vec!["plugin", "settings", "set", "hello"];
            full.extend(&args);
            let out = run(&env, &full, None).await;
            assert!(
                !out.ok && out.stderr.contains(message),
                "{args:?}: {}",
                out.stderr
            );
        }
        assert_eq!(setting_rows(&pool).await, 0);
        assert_eq!(audit(&pool).await.len(), 1, "only the install");
    })
    .await;
}

#[tokio::test]
async fn a_dry_run_checks_the_value_and_writes_nothing() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        install_configurable(&env).await;

        let out = run(
            &env,
            &[
                "plugin",
                "settings",
                "set",
                "hello",
                "unit",
                "characters",
                "--dry-run",
            ],
            None,
        )
        .await;
        assert!(out.ok && out.stdout.contains("would set"), "{}", out.stdout);
        let bad = run(
            &env,
            &[
                "plugin",
                "settings",
                "set",
                "hello",
                "unit",
                "x",
                "--dry-run",
            ],
            None,
        )
        .await;
        assert!(!bad.ok);
        assert_eq!(setting_rows(&pool).await, 0);
        let events: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events.outbox WHERE type = 'plugin.settings.changed'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((events, audit(&pool).await.len()), (0, 1));
    })
    .await;
}

#[tokio::test]
async fn a_course_setting_is_set_and_listed_for_its_course() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        install_configurable(&env).await;
        let owner = test_support::insert_user(&pool, "t@x.edu", "teacher").await;
        let course = uuid::Uuid::now_v7();
        sqlx::query("INSERT INTO teaching.courses (id, title, owner_id, created_by) VALUES ($1, 'C', $2, $2)")
            .bind(course).bind(owner).execute(&pool).await.unwrap();
        let course = course.to_string();

        let out = run(&env, &["plugin", "settings", "set", "hello", "max_words", "300", "--course", &course], None).await;
        assert!(out.ok, "{}", out.stderr);
        let listed = run(&env, &["plugin", "settings", "list", "hello", "--course", &course], None).await;
        assert!(listed.stdout.contains("max_words") && listed.stdout.contains("300"), "{}", listed.stdout);
        // The institution's settings are named, not shown, in a course's list.
        assert!(listed.stdout.contains("per institution: list without --course") && !listed.stdout.contains("(not set)"), "{}", listed.stdout);
        let meta = audit_meta(&pool, "plugin.setting.set").await;
        assert_eq!(meta["course_id"], course.as_str());
    })
    .await;
}

async fn a_course(pool: &PgPool, email: &str) -> uuid::Uuid {
    let owner = test_support::insert_user(pool, email, "teacher").await;
    let course = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO teaching.courses (id, title, owner_id, created_by) VALUES ($1, 'C', $2, $2)",
    )
    .bind(course)
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
    course
}

async fn courses_of_hello(pool: &PgPool) -> (bool, Vec<uuid::Uuid>) {
    let all: bool = sqlx::query_scalar(
        "SELECT all_courses FROM plugins.plugin_installs WHERE plugin_id = 'hello'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let list = sqlx::query_scalar(
        "SELECT course_id FROM plugins.install_courses c
         JOIN plugins.plugin_installs i ON i.id = c.install_id
         WHERE i.plugin_id = 'hello' ORDER BY course_id",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    (all, list)
}

#[tokio::test]
async fn the_courses_a_plugin_may_reach_are_chosen_at_install_and_changed_with_grant() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);
        let base = plugin(&["submissions:read"]).await;
        let (a, b) = (
            a_course(&pool, "a@x.edu").await,
            a_course(&pool, "b@x.edu").await,
        );

        // There is no default: with no answer the install stops and changes nothing.
        let silent = run(
            &env,
            &["plugin", "install", &base, "--grant", "none"],
            Some(""),
        )
        .await;
        assert!(!silent.ok);
        assert!(silent.stderr.contains("--courses"), "{}", silent.stderr);
        assert_eq!(installs(&pool).await, 0);
        let junk = run(
            &env,
            &[
                "plugin",
                "install",
                &base,
                "--grant",
                "none",
                "--courses",
                "everything",
            ],
            None,
        )
        .await;
        assert!(
            !junk.ok && junk.stderr.contains("not a course id"),
            "{}",
            junk.stderr
        );
        let ghost = run(
            &env,
            &[
                "plugin",
                "install",
                &base,
                "--grant",
                "none",
                "--courses",
                &uuid::Uuid::now_v7().to_string(),
            ],
            None,
        )
        .await;
        assert!(!ghost.ok, "a course that is not there stops the install");
        assert_eq!(installs(&pool).await, 0);

        // The question is asked when the flag is not given.
        let asked = run(
            &env,
            &["plugin", "install", &base, "--grant", "none"],
            Some(&format!("\n{a}, {b}\n")),
        )
        .await;
        assert!(asked.ok, "{}", asked.stderr);
        assert!(asked.stdout.contains("courses the plugin may reach"));
        let mut both = vec![a, b];
        both.sort();
        assert_eq!(courses_of_hello(&pool).await, (false, both));
        let meta = audit_meta(&pool, "plugin.install").await;
        assert_eq!(meta["courses"].as_array().unwrap().len(), 2);

        // Grant changes the courses with or without the scopes, and says what it did.
        let narrowed = run(
            &env,
            &[
                "plugin",
                "grant",
                "hello",
                "none",
                "--courses",
                &a.to_string(),
            ],
            None,
        )
        .await;
        assert!(narrowed.ok, "{}", narrowed.stderr);
        assert_eq!(courses_of_hello(&pool).await, (false, vec![a]));
        let meta = audit_meta(&pool, "plugin.grant").await;
        assert_eq!(meta["courses_to"], json!([a.to_string()]));
        let listed = run(&env, &["plugin", "list"], None).await;
        assert!(listed.stdout.contains(&a.to_string()), "{}", listed.stdout);
        // Without `--courses` they stay; the same words again change nothing.
        let kept = run(&env, &["plugin", "grant", "hello", "none"], None).await;
        assert!(
            kept.stdout.contains("nothing was changed"),
            "{}",
            kept.stdout
        );
        assert_eq!(courses_of_hello(&pool).await, (false, vec![a]));
        let everywhere = run(
            &env,
            &["plugin", "grant", "hello", "none", "--courses", "all"],
            None,
        )
        .await;
        assert!(everywhere.ok, "{}", everywhere.stderr);
        assert_eq!(courses_of_hello(&pool).await, (true, Vec::new()));
        let nowhere = run(
            &env,
            &["plugin", "grant", "hello", "none", "--courses", "none"],
            None,
        )
        .await;
        assert!(nowhere.ok, "{}", nowhere.stderr);
        assert_eq!(courses_of_hello(&pool).await, (false, Vec::new()));
    })
    .await;
}
