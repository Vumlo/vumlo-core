use access::Scope;
use events::{Event, process_once};
use serde_json::json;
use sqlx::PgPool;
use test_support::db_test;
use uuid::Uuid;
use vumlo_config::Secret;

use super::*;

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
    assert_eq!(SUBSCRIBABLE, ["submission.submitted", "grade.published"]);
}
