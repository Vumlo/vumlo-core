use crate::tests::db_test;
use access::PluginCaller;
use serde_json::json;
use sqlx::PgPool;
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
    install(
        &mut conn,
        &key(),
        "http://127.0.0.1:9",
        &hello(),
        granted,
        &access::CourseScope::All,
    )
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
        occurred_at: chrono::Utc::now(),
    }
}

#[test]
fn a_base_url_is_https_or_http_on_this_machine_or_a_container_and_carries_nothing_else() {
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
    assert_eq!(
        check_base_url("http://plugin-text-metrics:8000/").unwrap(),
        "http://plugin-text-metrics:8000",
        "the name of a container beside the core"
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
            &access::CourseScope::All,
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
        let again = install(
            &mut conn,
            &key(),
            "http://127.0.0.1:9",
            &hello(),
            &[],
            &access::CourseScope::All,
        )
        .await;
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

        let changed = grant(
            &mut conn,
            "hello",
            &[Scope::ReportsWrite],
            &access::CourseScope::All,
        )
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
            &access::CourseScope::All,
        )
        .await
        .unwrap();
        assert_eq!(all.scopes, [Scope::SubmissionsRead, Scope::ReportsWrite]);
        assert!(
            grant(&mut conn, "hello", &[], &access::CourseScope::All)
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
            grant(&mut conn, "nobody", &[], &access::CourseScope::All).await,
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
        install(
            &mut conn,
            &key(),
            "http://127.0.0.1:9",
            &grades,
            &[],
            &access::CourseScope::All,
        )
        .await
        .unwrap();
        let both = manifest("both", &[], &["submission.submitted", "grade.published"]);
        install(
            &mut conn,
            &key(),
            "http://127.0.0.1:9",
            &both,
            &[],
            &access::CourseScope::All,
        )
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

        let refused = put_report(
            &mut conn,
            &without,
            ReportSubject::Submission(submission),
            "x",
            "x",
            &json!({}),
        )
        .await;
        assert!(matches!(refused, Err(PluginsError::Forbidden(_))));

        // With the scope, but nothing to report on: the foreign key says so.
        let me = PluginCaller::for_install(id, "hello", &[Scope::ReportsWrite]);
        let gone = put_report(
            &mut conn,
            &me,
            ReportSubject::Submission(submission),
            "x",
            "x",
            &json!({}),
        )
        .await;
        assert!(matches!(gone, Err(PluginsError::SubjectGone)), "{gone:?}");
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

// ---------------------------------------------------------------------------------------------
// Settings (ADR-0072)
// ---------------------------------------------------------------------------------------------

fn configurable(id: &str) -> PluginManifest {
    let value = json!({
        "id": id, "version": "1.0.0", "api": "1",
        "scopes": ["settings:read"], "subscribes": ["plugin.settings.changed"],
        "settings": {
            "unit": {"type": "string", "enum": ["words", "characters"], "default": "words"},
            "api_token": {"type": "string", "secret": true},
            "tags": {"type": "array", "items": {"type": "string"}},
            "max_words": {"type": "number", "minimum": 1, "scope": "course"},
            "strict": {"type": "boolean", "default": false, "scope": "course"}
        }
    });
    parse_manifest(value.to_string().as_bytes()).unwrap()
}

async fn install_configurable(pool: &PgPool, id: &str) -> Installed {
    let mut conn = pool.acquire().await.unwrap();
    install(
        &mut conn,
        &key(),
        "http://127.0.0.1:9",
        &configurable(id),
        &[Scope::SettingsRead],
        &access::CourseScope::All,
    )
    .await
    .unwrap()
}

async fn a_course(pool: &PgPool) -> Uuid {
    let owner =
        test_support::insert_user(pool, &format!("{}@x.edu", Uuid::now_v7()), "teacher").await;
    let actor = Actor::for_tests(owner, common::Role::Teacher, Uuid::now_v7());
    let mut conn = pool.acquire().await.unwrap();
    let title = learning::teaching::CourseTitle::parse("Course").unwrap();
    learning::teaching::create_course(&mut conn, &actor, title, None)
        .await
        .unwrap()
        .id
}

async fn set(
    pool: &PgPool,
    plugin: &str,
    name: &str,
    value: Value,
    course: Option<Uuid>,
) -> Result<bool, PluginsError> {
    let mut conn = pool.acquire().await.unwrap();
    set_setting(&mut conn, &key(), plugin, name, value, course).await
}

/// What the plugin reads, as the plugin API would give it.
async fn read(pool: &PgPool, installed: &Installed, course: Option<Uuid>) -> Map<String, Value> {
    let caller = PluginCaller::for_install(
        installed.install.id,
        &installed.install.plugin_id,
        &installed.install.scopes,
    );
    let mut conn = pool.acquire().await.unwrap();
    settings_for_plugin(&mut conn, &caller, &key(), course)
        .await
        .unwrap()
}

async fn changes(pool: &PgPool) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT payload FROM events.outbox WHERE type = 'plugin.settings.changed' ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_setting_is_its_default_until_set_and_a_set_value_is_what_the_plugin_reads() {
    db_test(async |pool| {
        let installed = install_configurable(&pool, "conf").await;
        assert_eq!(
            read(&pool, &installed, None).await,
            json!({"unit": "words"}).as_object().unwrap().clone()
        );

        assert!(
            set(&pool, "conf", "unit", json!("characters"), None)
                .await
                .unwrap()
        );
        assert!(
            set(&pool, "conf", "tags", json!(["a", "b"]), None)
                .await
                .unwrap()
        );
        let values = read(&pool, &installed, None).await;
        assert_eq!(
            Value::Object(values),
            json!({"unit": "characters", "tags": ["a", "b"]})
        );

        // The administrator sees the same, with where each value comes from.
        let mut conn = pool.acquire().await.unwrap();
        let views = list_settings(&mut conn, "conf", None).await.unwrap();
        let unit = views.iter().find(|v| v.key == "unit").unwrap();
        assert_eq!(
            (unit.is_set, unit.value.clone()),
            (true, Some(json!("characters")))
        );
        assert!(
            unset_setting(&mut conn, "conf", "unit", None)
                .await
                .unwrap()
        );
        let unit = get_setting(&mut conn, "conf", "unit", None).await.unwrap();
        assert_eq!((unit.is_set, unit.value), (false, Some(json!("words"))));
        // Course settings are not in the institution's list.
        assert!(views.iter().all(|v| v.key != "max_words"));
    })
    .await;
}

#[tokio::test]
async fn a_secret_is_stored_sealed_and_only_the_plugin_reads_it_back() {
    db_test(async |pool| {
        let installed = install_configurable(&pool, "conf").await;
        assert!(
            set(&pool, "conf", "api_token", json!("tok-s3cret"), None)
                .await
                .unwrap()
        );

        let stored: String =
            sqlx::query_scalar("SELECT value::text FROM plugins.settings WHERE key = 'api_token'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!stored.contains("s3cret"), "{stored}");
        assert_eq!(
            read(&pool, &installed, None).await["api_token"],
            "tok-s3cret"
        );

        // The administrator is told it is set, never what it is.
        let mut conn = pool.acquire().await.unwrap();
        let view = get_setting(&mut conn, "conf", "api_token", None)
            .await
            .unwrap();
        assert_eq!((view.is_set, view.value), (true, None));
        assert!(
            !format!(
                "{:?}",
                list_settings(&mut conn, "conf", None).await.unwrap()
            )
            .contains("s3cret")
        );

        // The same token again is no change, although its sealed form would differ.
        assert!(
            !set(&pool, "conf", "api_token", json!("tok-s3cret"), None)
                .await
                .unwrap()
        );
        assert!(
            set(&pool, "conf", "api_token", json!("tok-other"), None)
                .await
                .unwrap()
        );
        assert_eq!(
            read(&pool, &installed, None).await["api_token"],
            "tok-other"
        );
    })
    .await;
}

#[tokio::test]
async fn reading_settings_needs_the_scope_and_a_plugin_reads_only_its_own() {
    db_test(async |pool| {
        let one = install_configurable(&pool, "one").await;
        let two = install_configurable(&pool, "two").await;
        set(&pool, "one", "api_token", json!("one-token"), None)
            .await
            .unwrap();

        let mut conn = pool.acquire().await.unwrap();
        let without = PluginCaller::for_install(one.install.id, "one", &[Scope::SubmissionsRead]);
        assert!(matches!(
            settings_for_plugin(&mut conn, &without, &key(), None).await,
            Err(PluginsError::Forbidden(_))
        ));
        assert!(!read(&pool, &two, None).await.contains_key("api_token"));
        assert_eq!(read(&pool, &one, None).await["api_token"], "one-token");
    })
    .await;
}

#[tokio::test]
async fn a_value_is_checked_against_the_spec_and_a_wrong_one_stores_and_announces_nothing() {
    db_test(async |pool| {
        install_configurable(&pool, "conf").await;
        for (name, value) in [
            ("unit", json!("furlongs")),
            ("unit", json!(3)),
            ("tags", json!("a")),
            ("api_token", json!("")),
        ] {
            let error = set(&pool, "conf", name, value, None).await.unwrap_err();
            assert!(
                matches!(error, PluginsError::BadSetting { .. }),
                "{name}: {error}"
            );
        }
        assert!(matches!(
            set(&pool, "conf", "nope", json!(1), None).await,
            Err(PluginsError::NoSuchSetting { .. })
        ));
        assert!(matches!(
            set(&pool, "missing", "unit", json!("words"), None).await,
            Err(PluginsError::NotFound(_))
        ));
        assert_eq!(count_settings(&pool).await, 0);
        assert!(changes(&pool).await.is_empty());
    })
    .await;
}

async fn count_settings(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM plugins.settings")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_course_setting_is_per_course_and_a_scope_is_not_mixed_up() {
    db_test(async |pool| {
        let installed = install_configurable(&pool, "conf").await;
        let (a, b) = (a_course(&pool).await, a_course(&pool).await);

        assert!(
            set(&pool, "conf", "max_words", json!(100), Some(a))
                .await
                .unwrap()
        );
        assert!(
            set(&pool, "conf", "max_words", json!(250), Some(b))
                .await
                .unwrap()
        );
        assert!(
            set(&pool, "conf", "max_words", json!(120), Some(a))
                .await
                .unwrap()
        );

        // One course's value is not another's; a course with none gets the default or nothing.
        assert_eq!(read(&pool, &installed, Some(a)).await["max_words"], 120);
        assert_eq!(read(&pool, &installed, Some(b)).await["max_words"], 250);
        let fresh = a_course(&pool).await;
        let values = read(&pool, &installed, Some(fresh)).await;
        assert!(!values.contains_key("max_words") && values["strict"] == false);
        // The institution's values come along, and no course key without a course.
        assert_eq!(values["unit"], "words");
        assert!(!read(&pool, &installed, None).await.contains_key("strict"));
        assert_eq!(count_settings(&pool).await, 2);

        // A course setting needs a course and an institution setting refuses one.
        assert!(matches!(
            set(&pool, "conf", "max_words", json!(1), None).await,
            Err(PluginsError::WrongScope { .. })
        ));
        assert!(matches!(
            set(&pool, "conf", "unit", json!("words"), Some(a)).await,
            Err(PluginsError::WrongScope { .. })
        ));
        // A course that is not there.
        assert!(matches!(
            set(&pool, "conf", "max_words", json!(1), Some(Uuid::now_v7())).await,
            Err(PluginsError::CourseGone)
        ));
    })
    .await;
}

#[tokio::test]
async fn a_change_is_announced_once_with_ids_only_and_only_to_its_plugin() {
    db_test(async |pool| {
        let one = install_configurable(&pool, "one").await;
        install_configurable(&pool, "two").await;
        let course = a_course(&pool).await;

        assert!(set(&pool, "one", "unit", json!("characters"), None).await.unwrap());
        assert!(!set(&pool, "one", "unit", json!("characters"), None).await.unwrap(), "same value");
        assert!(set(&pool, "one", "max_words", json!(10), Some(course)).await.unwrap());
        {
            let mut conn = pool.acquire().await.unwrap();
            assert!(unset_setting(&mut conn, "one", "max_words", Some(course)).await.unwrap());
            assert!(!unset_setting(&mut conn, "one", "max_words", Some(course)).await.unwrap());
        }

        let events = changes(&pool).await;
        assert_eq!(events.len(), 3, "{events:?}");
        assert_eq!(events[0], json!({"install_id": one.install.id, "course_id": null}));
        assert_eq!(events[1], json!({"install_id": one.install.id, "course_id": course}));
        assert!(!format!("{events:?}").contains("characters"));

        // Both plugins subscribed to the type; the change reaches the one it is about.
        let mut conn = pool.acquire().await.unwrap();
        let outbox = OutboxEvent {
            id: Uuid::now_v7(),
            kind: "plugin.settings.changed".to_owned(),
            version: 1,
            payload: events[0].clone(),
            occurred_at: chrono::Utc::now(),
        };
        assert_eq!(fan_out(&mut conn, &outbox).await.unwrap(), 1);
        let owner: String = sqlx::query_scalar(
            "SELECT i.plugin_id FROM plugins.deliveries d JOIN plugins.plugin_installs i ON i.id = d.install_id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(owner, "one");
    })
    .await;
}

#[tokio::test]
async fn removing_a_plugin_removes_its_settings() {
    db_test(async |pool| {
        install_configurable(&pool, "conf").await;
        set(&pool, "conf", "unit", json!("characters"), None)
            .await
            .unwrap();
        let mut conn = pool.acquire().await.unwrap();
        remove(&mut conn, "conf").await.unwrap();
        assert_eq!(count_settings(&pool).await, 0);
    })
    .await;
}

fn about(course: Uuid) -> OutboxEvent {
    OutboxEvent {
        id: Uuid::now_v7(),
        kind: "submission.submitted".to_owned(),
        version: 1,
        payload: json!({"course_id": course, "submission_id": Uuid::now_v7()}),
        occurred_at: chrono::Utc::now(),
    }
}

async fn install_in(pool: &PgPool, id: &str, courses: &CourseScope) -> Installed {
    let mut conn = pool.acquire().await.unwrap();
    install(
        &mut conn,
        &key(),
        "http://127.0.0.1:9",
        &manifest(id, &["submissions:read"], &["submission.submitted"]),
        &[Scope::SubmissionsRead],
        courses,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn an_install_keeps_the_courses_it_was_granted_and_grant_replaces_them() {
    db_test(async |pool| {
        let (a, b) = (a_course(&pool).await, a_course(&pool).await);
        let one = install_in(&pool, "one", &CourseScope::Only(vec![a])).await;
        assert_eq!(one.install.courses, CourseScope::Only(vec![a]));
        let mut conn = pool.acquire().await.unwrap();
        let read = get(&mut conn, "one").await.unwrap();
        assert_eq!(read.courses, CourseScope::Only(vec![a]));

        let widened = grant(
            &mut conn,
            "one",
            &[Scope::SubmissionsRead],
            &CourseScope::Only(vec![b, a]),
        )
        .await
        .unwrap();
        let CourseScope::Only(mut list) = widened.courses else {
            panic!("a list")
        };
        list.sort();
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(list, expected);

        let everything = grant(&mut conn, "one", &[], &CourseScope::All)
            .await
            .unwrap();
        assert_eq!(everything.courses, CourseScope::All);
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM plugins.install_courses")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0, "`all` keeps no list");
        let none = grant(&mut conn, "one", &[], &CourseScope::Only(Vec::new()))
            .await
            .unwrap();
        assert_eq!(none.courses, CourseScope::Only(Vec::new()));

        // A course that is not there refuses the grant, and changes nothing.
        let ghost = grant(
            &mut conn,
            "one",
            &[],
            &CourseScope::Only(vec![Uuid::now_v7()]),
        )
        .await;
        assert!(matches!(ghost, Err(PluginsError::CourseGone)), "{ghost:?}");
    })
    .await;
}

#[tokio::test]
async fn the_fan_out_sends_a_plugin_only_the_events_of_the_courses_it_reaches() {
    db_test(async |pool| {
        let (a, b) = (a_course(&pool).await, a_course(&pool).await);
        install_in(&pool, "everywhere", &CourseScope::All).await;
        install_in(&pool, "in-a", &CourseScope::Only(vec![a])).await;
        install_in(&pool, "nowhere", &CourseScope::Only(Vec::new())).await;
        let mut conn = pool.acquire().await.unwrap();

        assert_eq!(fan_out(&mut conn, &about(a)).await.unwrap(), 2);
        assert_eq!(fan_out(&mut conn, &about(b)).await.unwrap(), 1);
        let owed: Vec<String> = sqlx::query_scalar(
            "SELECT i.plugin_id FROM plugins.deliveries d
             JOIN plugins.plugin_installs i ON i.id = d.install_id
             ORDER BY i.plugin_id, d.id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(owed, ["everywhere", "everywhere", "in-a"]);
    })
    .await;
}

#[tokio::test]
async fn a_plugin_reads_the_settings_of_a_course_only_when_it_reaches_it() {
    db_test(async |pool| {
        let (a, b) = (a_course(&pool).await, a_course(&pool).await);
        // `for_courses` makes a caller named "test".
        install_configurable(&pool, "test").await;
        let mut conn = pool.acquire().await.unwrap();
        grant(
            &mut conn,
            "test",
            &[Scope::SettingsRead],
            &CourseScope::Only(vec![a]),
        )
        .await
        .unwrap();
        let caller = PluginCaller::for_courses(&[Scope::SettingsRead], &[a]);
        assert!(
            settings_for_plugin(&mut conn, &caller, &key(), Some(a))
                .await
                .is_ok()
        );
        let refused = settings_for_plugin(&mut conn, &caller, &key(), Some(b)).await;
        assert!(
            matches!(refused, Err(PluginsError::Forbidden(_))),
            "{refused:?}"
        );
        // The institution's own settings belong to no course, so a course grant does not hide them.
        assert!(
            settings_for_plugin(&mut conn, &caller, &key(), None)
                .await
                .is_ok()
        );
    })
    .await;
}
