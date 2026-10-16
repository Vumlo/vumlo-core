//! SQL only. Every function takes the connection it runs on and never begins a transaction.

use sqlx::PgConnection;
use uuid::Uuid;

use super::model::AuditEntry;

pub async fn insert_audit(
    conn: &mut PgConnection,
    actor_id: Uuid,
    entry: &AuditEntry,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO events.audit_log (id, actor_id, action, resource_type, resource_id, meta)
         VALUES ($1, $2, $3, $4, $5, $6)",
        Uuid::now_v7(),
        actor_id,
        entry.action,
        entry.resource_type,
        entry.resource_id,
        entry.meta
    )
    .execute(conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::identity::{self, DisplayName, Email, NewUser, Role};

    #[sqlx::test(migrator = "crate::app::MIGRATOR")]
    async fn an_audit_entry_commits_and_rolls_back_with_its_transaction(pool: sqlx::PgPool) {
        let hash = identity::hash_password("irrelevant").await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        let user = identity::create_user(
            &mut tx,
            NewUser {
                email: Email::parse("a@x.edu").unwrap(),
                display_name: DisplayName::parse("Admin").unwrap(),
                role: Role::Admin,
            },
            hash,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let resource_id = Uuid::now_v7();
        let entry = || AuditEntry {
            action: "course.publish",
            resource_type: "course",
            resource_id,
            meta: json!({"role": "teacher"}),
        };

        let mut rolled_back = pool.begin().await.unwrap();
        insert_audit(&mut rolled_back, user.id, &entry())
            .await
            .unwrap();
        rolled_back.rollback().await.unwrap();
        let count = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM events.audit_log"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);

        let mut tx = pool.begin().await.unwrap();
        insert_audit(&mut tx, user.id, &entry()).await.unwrap();
        tx.commit().await.unwrap();
        let row = sqlx::query!(
            "SELECT actor_id, action, resource_type, resource_id, meta FROM events.audit_log"
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.actor_id, user.id);
        assert_eq!(row.action, "course.publish");
        assert_eq!(row.resource_type, "course");
        assert_eq!(row.resource_id, resource_id);
        assert_eq!(row.meta, json!({"role": "teacher"}));
    }
}
