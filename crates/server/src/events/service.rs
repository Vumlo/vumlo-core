use sqlx::PgConnection;

use super::model::AuditEntry;
use super::repo;
use crate::actor::Actor;

#[derive(Debug, thiserror::Error)]
pub enum EventsError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// Records that `actor` did something, on the caller's connection so the entry commits or rolls
/// back with the change it describes. Callers decide what is audited (AGENTS §5); this does not
/// authorize anything.
pub async fn audit(
    conn: &mut PgConnection,
    actor: &Actor,
    entry: AuditEntry,
) -> Result<(), EventsError> {
    repo::insert_audit(conn, actor.user_id(), &entry).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;

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
        let actor = Actor::new(user.id, Role::Admin, Uuid::now_v7());
        let resource_id = Uuid::now_v7();
        let entry = || AuditEntry {
            action: "course.publish",
            resource_type: "course",
            resource_id,
            meta: json!({"role": "teacher"}),
        };

        let mut rolled_back = pool.begin().await.unwrap();
        audit(&mut rolled_back, &actor, entry()).await.unwrap();
        rolled_back.rollback().await.unwrap();
        let count = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM events.audit_log"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);

        let mut tx = pool.begin().await.unwrap();
        audit(&mut tx, &actor, entry()).await.unwrap();
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
