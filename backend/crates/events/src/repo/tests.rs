use serde_json::json;
use test_support::{db_test, insert_user};

use super::*;

#[tokio::test]
async fn an_audit_entry_commits_and_rolls_back_with_its_transaction() {
    db_test(async |pool| {
            let user_id = insert_user(&pool, "a@x.edu", "admin").await;
            let resource_id = Uuid::now_v7();
            let entry = || AuditEntry {
                action: "course.publish",
                resource_type: "course",
                resource_id,
                meta: json!({"role": "teacher"}),
            };

            let mut rolled_back = pool.begin().await.unwrap();
            insert_audit(&mut rolled_back, Some(user_id), "api", &entry())
                .await
                .unwrap();
            rolled_back.rollback().await.unwrap();
            let count = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM events.audit_log"#)
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(count, 0);

            let mut tx = pool.begin().await.unwrap();
            insert_audit(&mut tx, Some(user_id), "api", &entry())
                .await
                .unwrap();
            tx.commit().await.unwrap();
            let row = sqlx::query!(
                "SELECT actor_id, source, action, resource_type, resource_id, meta FROM events.audit_log"
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(row.actor_id, Some(user_id));
            assert_eq!(row.source, "api");
            assert_eq!(row.action, "course.publish");
            assert_eq!(row.resource_type, "course");
            assert_eq!(row.resource_id, resource_id);
            assert_eq!(row.meta, json!({"role": "teacher"}));

            // A CLI row has no actor; the table refuses an api row without one and a cli row with one.
            let mut tx = pool.begin().await.unwrap();
            insert_audit(&mut tx, None, "cli", &entry()).await.unwrap();
            tx.commit().await.unwrap();
            for (actor, source) in [(None, "api"), (Some(user_id), "cli"), (None, "web")] {
                let mut tx = pool.begin().await.unwrap();
                let refused = insert_audit(&mut tx, actor, source, &entry()).await;
                assert!(
                    matches!(&refused, Err(sqlx::Error::Database(db)) if db.code().as_deref() == Some("23514")),
                    "{actor:?} {source}: {refused:?}"
                );
            }
            let cli = sqlx::query_scalar!(
                r#"SELECT count(*) AS "n!" FROM events.audit_log WHERE source = 'cli' AND actor_id IS NULL"#
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(cli, 1);
        })
        .await;
}

#[tokio::test]
async fn an_event_commits_and_rolls_back_with_its_transaction_and_holds_ids_only() {
    db_test(async |pool| {
        let event = Event::GradePublished {
            assignment_id: Uuid::now_v7(),
            student_id: Uuid::now_v7(),
            course_id: Uuid::now_v7(),
        };

        let mut rolled_back = pool.begin().await.unwrap();
        insert_outbox(&mut rolled_back, &event).await.unwrap();
        rolled_back.rollback().await.unwrap();
        let count = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM events.outbox"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);

        let mut tx = pool.begin().await.unwrap();
        insert_outbox(&mut tx, &event).await.unwrap();
        tx.commit().await.unwrap();
        let row = sqlx::query!("SELECT type, version, payload, processed_at FROM events.outbox")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.r#type, "grade.published");
        assert_eq!(row.version, 1);
        assert!(row.processed_at.is_none());
        let Event::GradePublished {
            assignment_id,
            student_id,
            course_id,
        } = event
        else {
            panic!("the test built a grade.published event");
        };
        assert_eq!(
            row.payload,
            json!({
                "assignment_id": assignment_id,
                "student_id": student_id,
                "course_id": course_id,
            })
        );
    })
    .await;
}
