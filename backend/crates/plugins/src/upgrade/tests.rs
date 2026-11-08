use access::CourseScope;
use common::Role;
use serde_json::json;
use sqlx::PgPool;
use test_support::db_test;
use vumlo_config::Secret;

use super::*;
use crate::manifest::parse_manifest;
use crate::secret::SecretKey;

fn key() -> SecretKey {
    SecretKey::from_config(&Secret::new("0123456789abcdef0123456789abcdef")).unwrap()
}

fn v1() -> serde_json::Value {
    json!({
        "id": "tool", "version": "1.0.0", "api": "1",
        "subscribes": ["submission.submitted", "grade.published"],
        "scopes": ["submissions:read", "reports:write", "settings:read"],
        "ui": [{"slot": "assignment.tab", "title": "Tool", "path": "/ui"}],
        "settings": {
            "unit": {"type": "string", "enum": ["words", "characters"], "default": "words"},
            "limit": {"type": "number", "minimum": 1, "maximum": 100},
            "token": {"type": "string", "secret": true},
            "old": {"type": "string"},
            "per_course": {"type": "number", "scope": "course"}
        },
        "routes": [{"method": "GET", "path": "/api/a", "roles": ["teacher"]}]
    })
}

/// The second version: asks for a new scope and no longer for one, takes a new event and drops
/// one, tightens `limit`, forgets `old`, moves `per_course` to the institution, makes `unit` a secret.
fn v2() -> serde_json::Value {
    json!({
        "id": "tool", "version": "2.0.0", "api": "1",
        "subscribes": ["submission.submitted", "enrollment.changed"],
        "scopes": ["submissions:read", "settings:read", "grades:read"],
        "ui": [{"slot": "assignment.tab", "title": "Tool 2", "path": "/ui"}],
        "settings": {
            "unit": {"type": "string", "secret": true},
            "limit": {"type": "number", "minimum": 1, "maximum": 10},
            "token": {"type": "string", "secret": true},
            "per_course": {"type": "number"},
            "fresh": {"type": "boolean", "default": true}
        },
        "routes": [
            {"method": "GET", "path": "/api/a", "roles": ["teacher", "admin"]},
            {"method": "POST", "path": "/api/b", "roles": ["student"]}
        ]
    })
}

fn manifest(value: &serde_json::Value) -> PluginManifest {
    parse_manifest(value.to_string().as_bytes()).unwrap()
}

async fn installed(pool: &PgPool) -> crate::Installed {
    let mut conn = pool.acquire().await.unwrap();
    crate::install(
        &mut conn,
        &key(),
        "http://127.0.0.1:9",
        &manifest(&v1()),
        &[Scope::SubmissionsRead, Scope::ReportsWrite],
        &CourseScope::Only(Vec::new()),
    )
    .await
    .unwrap()
}

async fn set(pool: &PgPool, name: &str, value: serde_json::Value, course: Option<Uuid>) {
    let mut conn = pool.acquire().await.unwrap();
    crate::set_setting(&mut conn, &key(), "tool", name, value, course)
        .await
        .unwrap();
}

async fn a_course(pool: &PgPool) -> Uuid {
    let owner =
        test_support::insert_user(pool, &format!("{}@x.edu", Uuid::now_v7()), "teacher").await;
    let actor = access::Actor::for_tests(owner, Role::Teacher, Uuid::now_v7());
    let mut conn = pool.acquire().await.unwrap();
    let title = learning::teaching::CourseTitle::parse("Course").unwrap();
    learning::teaching::create_course(&mut conn, &actor, title, None)
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn the_plan_names_what_changed_and_what_needs_approving() {
    db_test(async |pool| {
        let first = installed(&pool).await.install;
        let plan = plan(&first, &manifest(&v2()));
        assert_eq!(
            (plan.from_version.as_str(), plan.to_version.as_str()),
            ("1.0.0", "2.0.0")
        );
        assert_eq!(plan.scopes_new, [Scope::GradesRead]);
        assert_eq!(plan.scopes_dropped, [Scope::ReportsWrite]);
        assert_eq!(plan.subscribes_added, ["enrollment.changed"]);
        assert_eq!(plan.subscribes_removed, ["grade.published"]);
        assert_eq!(plan.settings_added, ["fresh"]);
        assert_eq!(plan.settings_removed, ["old"]);
        let mut changed = plan.settings_changed.clone();
        changed.sort();
        assert_eq!(changed, ["limit", "per_course", "unit"]);
        assert_eq!(plan.routes_added, ["POST /api/b"]);
        assert_eq!(plan.routes_changed, ["GET /api/a"]);
        assert!(plan.routes_removed.is_empty() && plan.pages_changed && !plan.is_empty());
        // A manifest that says what the install holds changes nothing.
        let same = crate::upgrade::plan(&first, &manifest(&v1()));
        assert!(same.is_empty(), "{same:?}");
    })
    .await;
}

#[tokio::test]
async fn a_route_that_turns_public_is_shown_as_gone_and_new_so_that_it_is_not_missed() {
    db_test(async |pool| {
        let first = installed(&pool).await.install;
        let mut open = v1();
        open["routes"] = serde_json::json!([{"method": "GET", "path": "/api/a", "auth": "none"}]);
        let plan = plan(&first, &manifest(&open));
        assert_eq!(plan.routes_removed, ["GET /api/a"]);
        assert_eq!(plan.routes_added, ["GET /api/a (public)"]);
        assert!(!plan.is_empty());
    })
    .await;
}

#[tokio::test]
async fn an_upgrade_follows_the_manifest_and_grants_only_what_was_approved() {
    db_test(async |pool| {
        let before = installed(&pool).await;
        let course = a_course(&pool).await;
        let mut conn = pool.acquire().await.unwrap();
        crate::grant(
            &mut conn,
            "tool",
            &[Scope::SubmissionsRead, Scope::ReportsWrite],
            &CourseScope::Only(vec![course]),
        )
        .await
        .unwrap();
        drop(conn);
        // Values: one that stays fit, one that will not meet the new maximum, one for a setting
        // that goes, a secret that becomes plain, one that changes scope.
        set(&pool, "limit", json!(50), None).await;
        set(&pool, "old", json!("x"), None).await;
        set(&pool, "token", json!("s3cret"), None).await;
        set(&pool, "unit", json!("words"), None).await;
        set(&pool, "per_course", json!(7), Some(course)).await;

        let mut conn = pool.acquire().await.unwrap();
        let done = crate::upgrade(&mut conn, "tool", &manifest(&v2()), &[])
            .await
            .unwrap();
        // The install is the new manifest.
        let after = &done.after;
        assert_eq!(after.version, "2.0.0");
        assert_eq!(
            after.requested_scopes,
            [
                Scope::SubmissionsRead,
                Scope::SettingsRead,
                Scope::GradesRead
            ]
        );
        assert_eq!(
            after.subscribes,
            ["submission.submitted", "enrollment.changed"]
        );
        assert_eq!(after.routes.len(), 2);
        assert_eq!(after.ui[0].title, "Tool 2");
        assert!(after.settings.contains_key("fresh") && !after.settings.contains_key("old"));
        // What it held and still asks for stays; what it no longer asks for goes; the new scope
        // was not approved, so it is not granted.
        assert_eq!(after.scopes, [Scope::SubmissionsRead]);
        // The credential and the courses stay.
        assert_eq!(after.id, before.install.id);
        assert_eq!(after.secret_version, before.install.secret_version);
        assert_eq!(after.courses, CourseScope::Only(vec![course]));
        // The token still opens it: only its hash is in the row, unchanged.
        let by_token = crate::repo::find_enabled_by_token(&mut conn, &before.token.hash())
            .await
            .unwrap();
        assert!(by_token.is_some());

        // Values: `limit` (50 > 10), `old` (gone), `token` (still a secret: kept), `unit` (was
        // plain, now a secret: dropped), `per_course` (changed scope: dropped).
        let mut dropped: Vec<_> = done.dropped.iter().map(|d| d.key.as_str()).collect();
        dropped.sort();
        assert_eq!(dropped, ["limit", "old", "per_course", "unit"]);
        let left = crate::repo::all_settings(&mut conn, after.id)
            .await
            .unwrap();
        assert_eq!(
            left.iter().map(|v| v.1.as_str()).collect::<Vec<_>>(),
            ["token"]
        );
        // The plugin is told its settings changed, once, for the whole institution.
        let events: Vec<serde_json::Value> = sqlx::query_scalar(
            "SELECT payload FROM events.outbox WHERE type = 'plugin.settings.changed'
             ORDER BY id DESC LIMIT 1",
        )
        .fetch_all(&mut *conn)
        .await
        .unwrap();
        assert_eq!(events[0]["install_id"], json!(after.id));

        // A second look at the same manifest has nothing to do.
        assert!(crate::upgrade::plan(after, &manifest(&v2())).is_empty());
    })
    .await;
}

#[tokio::test]
async fn a_new_scope_is_granted_when_it_is_approved_and_only_then_and_only_if_it_is_new() {
    db_test(async |pool| {
        installed(&pool).await;
        let mut conn = pool.acquire().await.unwrap();
        // Not newly asked for: reports:write was asked for before and is simply kept; a scope the
        // plugin does not ask for at all cannot be granted.
        let refused =
            crate::upgrade(&mut conn, "tool", &manifest(&v2()), &[Scope::ReportsWrite]).await;
        assert!(
            matches!(refused, Err(PluginsError::NotRequested(ref s)) if s == "reports:write"),
            "{refused:?}"
        );
        let refused =
            crate::upgrade(&mut conn, "tool", &manifest(&v2()), &[Scope::UsersContact]).await;
        assert!(matches!(refused, Err(PluginsError::NotRequested(_))));
        assert_eq!(
            crate::service::get(&mut conn, "tool")
                .await
                .unwrap()
                .version,
            "1.0.0",
            "nothing changed"
        );

        let done = crate::upgrade(&mut conn, "tool", &manifest(&v2()), &[Scope::GradesRead])
            .await
            .unwrap();
        assert_eq!(
            done.after.scopes,
            [Scope::SubmissionsRead, Scope::GradesRead]
        );
        // A scope that was asked for and left ungranted stays ungranted when it is asked for again.
        let mut again = v2();
        again["scopes"] = json!([
            "submissions:read",
            "settings:read",
            "grades:read",
            "reports:write"
        ]);
        let done = crate::upgrade(&mut conn, "tool", &manifest(&again), &[])
            .await
            .unwrap();
        assert!(
            !done.after.scopes.contains(&Scope::ReportsWrite),
            "{:?}",
            done.after.scopes
        );
        assert_eq!(
            done.plan.scopes_new,
            [Scope::ReportsWrite],
            "it was dropped, so it is new again"
        );
    })
    .await;
}

#[tokio::test]
async fn another_plugins_manifest_is_not_taken() {
    db_test(async |pool| {
        installed(&pool).await;
        let mut other = v2();
        other["id"] = json!("someone-else");
        let mut conn = pool.acquire().await.unwrap();
        let refused = crate::upgrade(&mut conn, "tool", &manifest(&other), &[]).await;
        assert!(
            matches!(refused, Err(PluginsError::Manifest(_))),
            "{refused:?}"
        );
        assert!(matches!(
            crate::upgrade(&mut conn, "nobody", &manifest(&v2()), &[]).await,
            Err(PluginsError::NotFound(_))
        ));
    })
    .await;
}
