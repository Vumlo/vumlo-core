//! `vumlo-cli facts rebuild` (ADR-0093): the facts the access rules read are made again from the
//! submissions and grades, and the run is audited.

mod common;

use common::{Env, audit, audit_meta, db_test, migrate_modules};
use sqlx::PgPool;

async fn count(pool: &PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

#[tokio::test]
async fn a_rebuild_restores_a_deleted_projection_and_is_audited() {
    db_test(async |pool| {
        migrate_modules(&pool).await;
        let env = Env::new(&pool);
        let seeded = env.run(&["seed", "--demo"]).await;
        assert!(seeded.ok, "{}", seeded.stderr);
        assert_eq!(count(&pool, "SELECT count(*) FROM content.facts").await, 3);
        sqlx::query("DELETE FROM content.facts")
            .execute(&pool)
            .await
            .unwrap();

        let rebuilt = env.run(&["facts", "rebuild"]).await;

        assert!(rebuilt.ok, "{}", rebuilt.stderr);
        assert_eq!(count(&pool, "SELECT count(*) FROM content.facts").await, 3);
        assert_eq!(
            audit(&pool).await,
            [
                ("demo.seed".into(), "cli".into(), true, "course".into()),
                (
                    "facts.rebuild".into(),
                    "cli".into(),
                    true,
                    "content_facts".into()
                ),
            ]
        );
        assert_eq!(
            audit_meta(&pool, "facts.rebuild").await["course_id"],
            serde_json::Value::Null
        );
    })
    .await;
}

#[tokio::test]
async fn a_rebuild_of_one_course_leaves_the_others() {
    db_test(async |pool| {
        migrate_modules(&pool).await;
        let env = Env::new(&pool);
        assert!(env.run(&["seed", "--demo"]).await.ok);
        let course: uuid::Uuid = sqlx::query_scalar("SELECT id FROM teaching.courses")
            .fetch_one(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM content.facts")
            .execute(&pool)
            .await
            .unwrap();

        let other = env
            .run(&[
                "facts",
                "rebuild",
                "--course",
                &uuid::Uuid::now_v7().to_string(),
            ])
            .await;
        assert!(other.ok, "{}", other.stderr);
        assert_eq!(count(&pool, "SELECT count(*) FROM content.facts").await, 0);

        let this = env
            .run(&["facts", "rebuild", "--course", &course.to_string()])
            .await;
        assert!(this.ok, "{}", this.stderr);
        assert_eq!(count(&pool, "SELECT count(*) FROM content.facts").await, 3);
        assert_eq!(
            audit_meta_of(&pool, &course).await,
            serde_json::json!({"course_id": course})
        );
    })
    .await;
}

async fn audit_meta_of(pool: &PgPool, course: &uuid::Uuid) -> serde_json::Value {
    sqlx::query_scalar(
        "SELECT meta FROM events.audit_log WHERE action = 'facts.rebuild' AND resource_id = $1",
    )
    .bind(course)
    .fetch_one(pool)
    .await
    .unwrap()
}
