use access::PluginCaller;
use serde_json::json;
use sqlx::PgPool;
use test_support::db_test;
use vumlo_config::Secret;

use super::*;
use crate::manifest::parse_manifest;

fn key() -> SecretKey {
    SecretKey::from_config(&Secret::new("0123456789abcdef0123456789abcdef")).unwrap()
}

fn manifest(id: &str, scopes: &[&str], subscribes: &[&str]) -> PluginManifest {
    let value = json!({
        "id": id, "version": "1.0.0", "api": "1",
        "scopes": scopes, "subscribes": subscribes,
    });
    parse_manifest(value.to_string().as_bytes()).unwrap()
}

fn hello() -> PluginManifest {
    manifest(
        "hello",
        &["submissions:read", "reports:write"],
        &["submission.submitted"],
    )
}

async fn install_hello(pool: &PgPool, granted: &[Scope]) -> Installed {
    let mut conn = pool.acquire().await.unwrap();
    install(&mut conn, &key(), "http://127.0.0.1:9", &hello(), granted)
        .await
        .unwrap()
}

async fn count(pool: &PgPool, table: &str) -> i64 {
    let sql = match table {
        "installs" => "SELECT count(*) FROM plugins.plugin_installs",
        "deliveries" => "SELECT count(*) FROM plugins.deliveries",
        _ => "SELECT count(*) FROM plugins.reports",
    };
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

fn event(kind: &str) -> OutboxEvent {
    OutboxEvent {
        id: Uuid::now_v7(),
        kind: kind.to_owned(),
        version: 1,
        payload: json!({"submission_id": Uuid::now_v7()}),
    }
}

#[test]
fn a_base_url_is_https_or_http_on_this_machine_and_carries_nothing_else() {
    assert_eq!(
        check_base_url("https://tools.example.edu/p/").unwrap(),
        "https://tools.example.edu/p"
    );
    assert_eq!(
        check_base_url("http://127.0.0.1:9000").unwrap(),
        "http://127.0.0.1:9000"
    );
    assert_eq!(
        check_base_url("http://localhost:9000/").unwrap(),
        "http://localhost:9000"
    );
    for bad in [
        "http://tools.example.edu",
        "ftp://tools.example.edu",
        "tools.example.edu",
        "https://user:pw@tools.example.edu",
        "https://tools.example.edu?key=1",
        "https://",
    ] {
        assert!(check_base_url(bad).is_err(), "{bad}");
    }
}

#[tokio::test]
async fn an_install_stores_the_hash_of_the_token_and_only_the_scopes_that_were_approved() {
    db_test(async |pool| {
        let installed = install_hello(&pool, &[Scope::SubmissionsRead]).await;

        assert!(installed.token.expose().starts_with("vplg_"));
        assert_eq!(installed.install.scopes, [Scope::SubmissionsRead]);
        assert_eq!(
            installed.install.requested_scopes,
            [Scope::SubmissionsRead, Scope::ReportsWrite]
        );
        assert_eq!(installed.install.status, InstallStatus::Enabled);
        assert_eq!(
            installed.secret,
            derive_secret(&key(), installed.install.id, 1).unwrap()
        );

        // Neither the token nor the secret is anywhere in the row; the hash of the token is.
        let row: String =
            sqlx::query_scalar("SELECT row_to_json(t)::text FROM plugins.plugin_installs t")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!row.contains(installed.token.expose()) && !row.contains(&installed.secret));
        let stored: Vec<u8> = sqlx::query_scalar("SELECT token_hash FROM plugins.plugin_installs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored, installed.token.hash());
        assert_eq!(format!("{:?}", installed.token), "PluginToken(***)");
        assert!(!format!("{installed:?}").contains(&installed.secret));
    })
    .await;
}

#[tokio::test]
async fn a_scope_the_plugin_did_not_ask_for_cannot_be_granted() {
    db_test(async |pool| {
        let mut conn = pool.acquire().await.unwrap();
        let reports_only = manifest("quiet", &["reports:write"], &[]);

        let refused = install(
            &mut conn,
            &key(),
            "http://127.0.0.1:9",
            &reports_only,
            &[Scope::SubmissionsRead],
        )
        .await
        .unwrap_err();

        assert!(
            matches!(refused, PluginsError::NotRequested(ref s) if s == "submissions:read"),
            "{refused}"
        );
        assert_eq!(count(&pool, "installs").await, 0);
    })
    .await;
}

#[tokio::test]
async fn a_plugin_may_be_granted_nothing_and_an_id_is_installed_once() {
    db_test(async |pool| {
        let none = install_hello(&pool, &[]).await;
        assert!(none.install.scopes.is_empty());

        let mut conn = pool.acquire().await.unwrap();
        let again = install(&mut conn, &key(), "http://127.0.0.1:9", &hello(), &[]).await;
        assert!(matches!(again, Err(PluginsError::AlreadyInstalled(ref id)) if id == "hello"));
        assert_eq!(count(&pool, "installs").await, 1);
    })
    .await;
}

#[tokio::test]
async fn grant_replaces_the_scopes_within_what_was_asked_for() {
    db_test(async |pool| {
        install_hello(&pool, &[Scope::SubmissionsRead]).await;
        let mut conn = pool.acquire().await.unwrap();

        let changed = grant(&mut conn, "hello", &[Scope::ReportsWrite])
            .await
            .unwrap();
        assert_eq!(changed.scopes, [Scope::ReportsWrite]);
        let all = grant(
            &mut conn,
            "hello",
            &[
                Scope::ReportsWrite,
                Scope::SubmissionsRead,
                Scope::ReportsWrite,
            ],
        )
        .await
        .unwrap();
        assert_eq!(all.scopes, [Scope::SubmissionsRead, Scope::ReportsWrite]);
        assert!(
            grant(&mut conn, "hello", &[])
                .await
                .unwrap()
                .scopes
                .is_empty()
        );

        // The database refuses a grant beyond the request even if the service did not.
        let beyond = sqlx::query("UPDATE plugins.plugin_installs SET scopes = '{users:read}'")
            .execute(&pool)
            .await;
        assert!(beyond.is_err());
        assert!(matches!(
            grant(&mut conn, "nobody", &[]).await,
            Err(PluginsError::NotFound(_))
        ));
    })
    .await;
}

#[tokio::test]
async fn the_fan_out_owes_each_enabled_subscriber_once() {
    db_test(async |pool| {
        install_hello(&pool, &[]).await;
        let mut conn = pool.acquire().await.unwrap();
        let grades = manifest("grades", &[], &["grade.published"]);
        install(&mut conn, &key(), "http://127.0.0.1:9", &grades, &[])
            .await
            .unwrap();
        let both = manifest("both", &[], &["submission.submitted", "grade.published"]);
        install(&mut conn, &key(), "http://127.0.0.1:9", &both, &[])
            .await
            .unwrap();

        let submitted = event("submission.submitted");
        assert_eq!(
            fan_out(&mut conn, &submitted).await.unwrap(),
            2,
            "hello and both"
        );
        assert_eq!(
            fan_out(&mut conn, &submitted).await.unwrap(),
            0,
            "a repeat owes nothing"
        );
        assert_eq!(
            fan_out(&mut conn, &event("user.password_reset_requested"))
                .await
                .unwrap(),
            0
        );
        assert_eq!(count(&pool, "deliveries").await, 2);

        set_status(&mut conn, "hello", InstallStatus::Disabled)
            .await
            .unwrap();
        assert_eq!(
            fan_out(&mut conn, &event("submission.submitted"))
                .await
                .unwrap(),
            1,
            "only both"
        );
        set_status(&mut conn, "hello", InstallStatus::Enabled)
            .await
            .unwrap();
        assert_eq!(
            fan_out(&mut conn, &event("submission.submitted"))
                .await
                .unwrap(),
            2
        );
    })
    .await;
}

#[tokio::test]
async fn removing_a_plugin_removes_its_deliveries() {
    // Its reports go too; that needs real submitted attempts, so it is in tests/plugins.rs.
    db_test(async |pool| {
        install_hello(&pool, &[Scope::ReportsWrite]).await;
        let mut conn = pool.acquire().await.unwrap();
        fan_out(&mut conn, &event("submission.submitted"))
            .await
            .unwrap();
        assert_eq!(count(&pool, "deliveries").await, 1);

        let removed = remove(&mut conn, "hello").await.unwrap();

        assert_eq!(removed.plugin_id, "hello");
        assert_eq!(
            (
                count(&pool, "installs").await,
                count(&pool, "deliveries").await
            ),
            (0, 0)
        );
        assert!(matches!(
            remove(&mut conn, "hello").await,
            Err(PluginsError::NotFound(_))
        ));
    })
    .await;
}

#[tokio::test]
async fn a_report_needs_the_scope_and_one_for_an_attempt_that_is_not_there_is_refused() {
    // A report that is stored, replaced and removed is in tests/plugins.rs, against real attempts.
    db_test(async |pool| {
        let installed = install_hello(&pool, &[Scope::ReportsWrite]).await;
        let mut conn = pool.acquire().await.unwrap();
        let submission = Uuid::now_v7();
        let id = installed.install.id;
        let without = PluginCaller::for_install(id, "hello", &[Scope::SubmissionsRead]);

        let refused = put_report(&mut conn, &without, submission, "x", &json!({})).await;
        assert!(matches!(refused, Err(PluginsError::Forbidden(_))));

        // With the scope, but nothing to report on: the foreign key says so.
        let me = PluginCaller::for_install(id, "hello", &[Scope::ReportsWrite]);
        let gone = put_report(&mut conn, &me, submission, "x", &json!({})).await;
        assert!(
            matches!(gone, Err(PluginsError::SubmissionGone)),
            "{gone:?}"
        );
        assert_eq!(count(&pool, "reports").await, 0);
    })
    .await;
}

#[tokio::test]
async fn list_counts_what_is_pending_and_what_is_dead() {
    db_test(async |pool| {
        install_hello(&pool, &[]).await;
        let mut conn = pool.acquire().await.unwrap();
        fan_out(&mut conn, &event("submission.submitted"))
            .await
            .unwrap();
        fan_out(&mut conn, &event("submission.submitted"))
            .await
            .unwrap();
        sqlx::query(
            "UPDATE plugins.deliveries SET failed_at = now()
             WHERE id = (SELECT id FROM plugins.deliveries ORDER BY id LIMIT 1)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let summary = list(&mut conn).await.unwrap();

        assert_eq!(summary.len(), 1);
        assert_eq!((summary[0].pending, summary[0].failed), (1, 1));
    })
    .await;
}
