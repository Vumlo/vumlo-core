use access::Scope;
use events::{Event, process_once};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use vumlo_config::Secret;

use super::*;

/// `db_test` for the unit tests of this crate: what they emit needs a registry of subscribers
/// installed, as the server does at start (ADR-0093). `plugins` has no subscribers of its own.
pub(crate) async fn db_test(body: impl AsyncFnOnce(PgPool)) {
    events::testing::ensure_installed(events::Subscribers::new);
    test_support::db_test(body).await;
}

fn key() -> SecretKey {
    SecretKey::from_config(&Secret::new("0123456789abcdef0123456789abcdef")).unwrap()
}

async fn install_one(pool: &PgPool, id: &str, subscribes: &[&str]) {
    let manifest = parse_manifest(
        json!({"id": id, "version": "1", "api": "1", "subscribes": subscribes, "scopes": ["submissions:read"]})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    install(
        &mut conn,
        &key(),
        "http://127.0.0.1:9",
        &manifest,
        &[Scope::SubmissionsRead],
        &access::CourseScope::All,
    )
    .await
    .unwrap();
}

async fn deliveries(pool: &PgPool) -> Vec<(String, String, serde_json::Value)> {
    sqlx::query_as(
        "SELECT i.plugin_id, d.event_type, d.data FROM plugins.deliveries d
         JOIN plugins.plugin_installs i ON i.id = d.install_id ORDER BY i.plugin_id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn an_event_in_the_outbox_owes_a_delivery_to_each_plugin_that_asked_for_it() {
    db_test(async |pool| {
        install_one(&pool, "reader", &["submission.submitted"]).await;
        install_one(&pool, "grader", &["grade.published"]).await;
        let (assignment_id, student_id, course_id, submission_id) = (
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
        );
        let mut conn = pool.acquire().await.unwrap();
        events::emit(
            &mut conn,
            Event::SubmissionSubmitted {
                assignment_id,
                student_id,
                course_id,
                submission_id,
            },
        )
        .await
        .unwrap();
        drop(conn);

        process_once(&pool, &event_handlers(pool.clone()))
            .await
            .unwrap();

        let owed = deliveries(&pool).await;
        assert_eq!(owed.len(), 1);
        assert_eq!(
            (owed[0].0.as_str(), owed[0].1.as_str()),
            ("reader", "submission.submitted")
        );
        // The payload is the ids of the outbox event and nothing else (AGENTS section 7).
        assert_eq!(
            owed[0].2,
            json!({
                "assignment_id": assignment_id, "student_id": student_id,
                "course_id": course_id, "submission_id": submission_id
            })
        );
    })
    .await;
}

#[test]
fn the_events_a_plugin_may_subscribe_to_are_events_that_exist_and_hold_ids_only() {
    assert_eq!(
        SUBSCRIBABLE,
        [
            "submission.submitted",
            "grade.published",
            "plugin.settings.changed",
            "assignment.created",
            "assignment.updated",
            "assignment.deadline_changed",
            "enrollment.changed",
            "quiz.attempt_submitted",
            "announcement.published",
        ]
    );
}

#[test]
fn an_internal_event_is_not_one_a_plugin_may_ask_for() {
    for internal in ["grade.publication_changed", "group.members_changed"] {
        assert!(!SUBSCRIBABLE.contains(&internal), "{internal}");
    }
}

#[test]
fn a_route_matches_a_path_of_plain_segments_and_names_its_parameters() {
    let route = |path: &str| crate::model::RouteSpec {
        method: "GET".to_owned(),
        auth: crate::model::RouteAuth::Session,
        path: path.to_owned(),
        roles: vec![],
        course_roles: vec![],
    };
    let r = route("/api/summary/{course_id}");
    assert_eq!(
        r.matches("/api/summary/abc-1"),
        Some(vec![("course_id", "abc-1")])
    );
    assert_eq!(
        route("/a/{x}/b/{y}").matches("/a/1/b/2"),
        Some(vec![("x", "1"), ("y", "2")])
    );
    assert_eq!(route("/a").matches("/a"), Some(vec![]));
    for path in [
        "/api/summary",
        "/api/summary/",
        "/api/summary/a/b",
        "/api/other/a",
        "/api/summary/..",
        "/api/summary/.",
        "/api/summary/a%2fb",
        "/api/summary/a b",
        "/api/summary/a;b",
        "//api/summary/a",
        "api/summary/a",
    ] {
        assert_eq!(r.matches(path), None, "{path}");
    }
    // A literal is exact, also in case.
    assert_eq!(route("/Api").matches("/api"), None);
}

#[test]
fn a_segment_is_plain_when_it_cannot_climb_or_escape() {
    use crate::model::is_plain_segment;
    for ok in [
        "a",
        "A1",
        "a.b",
        "a_b",
        "a~b",
        "a-b",
        "...x",
        "018f0000-0000-7000-8000-000000000003",
    ] {
        assert!(is_plain_segment(ok), "{ok}");
    }
    for bad in [
        "", ".", "..", "a/b", "a%2e", "a b", "a?b", "a#b", "a\\b", "é", "a;b", "a:b",
    ] {
        assert!(!is_plain_segment(bad), "{bad}");
    }
}
