//! SQL only. Every function takes the connection it runs on and never begins a transaction.

use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{AuditEntry, AuditFilter, AuditRow, Event, OutboxEvent};

type Meta = sqlx::types::Json<serde_json::Map<String, serde_json::Value>>;

/// Audit rows newest first, with the actor's name and what the target is called now. The kind is
/// derived from the action here and nowhere else (ADR-0053).
///
/// ponytail: the filter by time scans `audit_log` without an index on `at`; add one when a long
/// period of a busy installation becomes slow.
pub async fn list_audit(
    conn: &mut PgConnection,
    filter: &AuditFilter,
    pattern: Option<&str>,
    before: Option<Uuid>,
    limit: i64,
) -> Result<Vec<AuditRow>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT log.id AS "id!", log.at AS "at!", log.source AS "source!", log.action AS "action!",
                  log.resource_type AS "resource_type!", log.resource_id AS "resource_id!",
                  log.meta AS "meta!: Meta",
                  log.actor_id, log.actor_name AS "actor_name?", log.actor_surname AS "actor_surname?",
                  log.kind AS "kind!", log.label AS "label?"
           FROM (
               SELECT l.id, l.at, l.source, l.action, l.resource_type, l.resource_id, l.meta,
                      l.actor_id, a.name AS actor_name, a.surname AS actor_surname,
                      CASE WHEN l.action LIKE 'user.%' THEN 'accounts'
                           WHEN l.action = 'course.owner.transfer' THEN 'owners'
                           WHEN l.action LIKE 'course.member.%' THEN 'teachers'
                           WHEN l.action LIKE 'group.%' OR l.action LIKE 'subgroup.%' THEN 'groups'
                           WHEN l.action LIKE 'course.%' THEN 'courses'
                           WHEN l.action LIKE 'content.%' THEN 'content'
                           WHEN l.action LIKE 'grade.%' THEN 'grades'
                           WHEN l.action LIKE 'term.%' THEN 'terms'
                           ELSE 'other' END AS kind,
                      CASE l.resource_type
                          WHEN 'user' THEN ru.name || ' ' || ru.surname
                          WHEN 'course' THEN concat_ws(' ', NULLIF(rc.code, ''), rc.title)
                          WHEN 'group' THEN rg.name
                          WHEN 'subgroup' THEN rs.name
                          WHEN 'term' THEN rt.name
                          WHEN 'item' THEN ri.title
                          WHEN 'assignment' THEN ri.title
                          WHEN 'module' THEN rm.title
                      END AS label
               FROM events.audit_log l
               LEFT JOIN identity.users a ON a.id = l.actor_id
               LEFT JOIN identity.users ru ON l.resource_type = 'user' AND ru.id = l.resource_id
               LEFT JOIN teaching.courses rc ON l.resource_type = 'course' AND rc.id = l.resource_id
               LEFT JOIN org.groups rg ON l.resource_type = 'group' AND rg.id = l.resource_id
               LEFT JOIN org.subgroups rs ON l.resource_type = 'subgroup' AND rs.id = l.resource_id
               LEFT JOIN org.terms rt ON l.resource_type = 'term' AND rt.id = l.resource_id
               LEFT JOIN content.items ri
                      ON l.resource_type IN ('item', 'assignment') AND ri.id = l.resource_id
               LEFT JOIN content.modules rm ON l.resource_type = 'module' AND rm.id = l.resource_id
               WHERE l.at >= now() - make_interval(days => $2)
                 AND ($3::uuid IS NULL OR l.id < $3)
           ) log
           WHERE ($1::text IS NULL OR log.kind = $1)
             AND ($4::text IS NULL
                  OR (log.actor_name || ' ' || log.actor_surname) ILIKE $4
                  OR log.action ILIKE $4 OR log.label ILIKE $4)
           ORDER BY log.id DESC
           LIMIT $5"#,
        filter.kind.as_deref(),
        filter.days,
        before,
        pattern,
        limit
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| AuditRow {
            id: row.id,
            at: row.at,
            source: row.source,
            kind: row.kind,
            action: row.action,
            actor: row
                .actor_id
                .zip(row.actor_name)
                .zip(row.actor_surname)
                .map(|((id, name), surname)| (id, name, surname)),
            resource_type: row.resource_type,
            resource_id: row.resource_id,
            resource_label: row.label,
            meta: row.meta.0,
        })
        .collect())
}

/// `actor_id` is `None` exactly when `source` is `cli` (a check of the table).
pub async fn insert_audit(
    conn: &mut PgConnection,
    actor_id: Option<Uuid>,
    source: &str,
    entry: &AuditEntry,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO events.audit_log (id, actor_id, source, action, resource_type, resource_id, meta)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        Uuid::now_v7(),
        actor_id,
        source,
        entry.action,
        entry.resource_type,
        entry.resource_id,
        entry.meta
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn insert_outbox(conn: &mut PgConnection, event: &Event) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO events.outbox (id, type, version, payload) VALUES ($1, $2, $3, $4)",
        Uuid::now_v7(),
        event.kind(),
        event.version(),
        event.payload()
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Claims up to `limit` due rows with a lease: `attempts` goes up and `run_after` moves
/// `lease_secs` forward, so a crashed worker's rows come back later (ADR-0045). Oldest first.
pub async fn claim(
    conn: &mut PgConnection,
    limit: i64,
    lease_secs: f64,
) -> Result<Vec<(OutboxEvent, i32)>, sqlx::Error> {
    let mut rows = sqlx::query!(
        r#"UPDATE events.outbox
           SET attempts = attempts + 1, run_after = now() + make_interval(secs => $2)
           WHERE id IN (
               SELECT id FROM events.outbox
               WHERE processed_at IS NULL AND failed_at IS NULL AND run_after <= now()
               ORDER BY id LIMIT $1 FOR UPDATE SKIP LOCKED)
           RETURNING id, type AS kind, version, payload, attempts"#,
        limit,
        lease_secs
    )
    .fetch_all(conn)
    .await?;
    rows.sort_by_key(|row| row.id);
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                OutboxEvent {
                    id: row.id,
                    kind: row.kind,
                    version: row.version,
                    payload: row.payload,
                },
                row.attempts,
            )
        })
        .collect())
}

pub async fn mark_processed(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE events.outbox SET processed_at = now(), last_error = NULL WHERE id = $1",
        id
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Records a failed attempt. `give_up` turns the row into a dead letter; otherwise it is due again
/// in `retry_secs`.
pub async fn mark_failed(
    conn: &mut PgConnection,
    id: Uuid,
    error: &str,
    give_up: bool,
    retry_secs: f64,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE events.outbox
         SET last_error = $2,
             failed_at = CASE WHEN $3 THEN now() END,
             run_after = now() + make_interval(secs => $4)
         WHERE id = $1",
        id,
        error,
        give_up,
        retry_secs
    )
    .execute(conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::test_support::db_test;
    use serde_json::json;

    use super::*;
    use crate::identity::{self, Email, NewUser, PersonName, Role};

    #[tokio::test]
    async fn an_audit_entry_commits_and_rolls_back_with_its_transaction() {
        db_test(async |pool| {
            let hash = identity::hash_password("irrelevant").await.unwrap();
            let mut tx = pool.begin().await.unwrap();
            let user = identity::create_user_unaudited_for_tests(
                &mut tx,
                NewUser {
                    email: Email::parse("a@x.edu").unwrap(),
                    name: PersonName::name("Admin").unwrap(),
                    surname: PersonName::surname("Admin").unwrap(),
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
            insert_audit(&mut rolled_back, Some(user.id), "api", &entry())
                .await
                .unwrap();
            rolled_back.rollback().await.unwrap();
            let count = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM events.audit_log"#)
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(count, 0);

            let mut tx = pool.begin().await.unwrap();
            insert_audit(&mut tx, Some(user.id), "api", &entry())
                .await
                .unwrap();
            tx.commit().await.unwrap();
            let row = sqlx::query!(
                "SELECT actor_id, source, action, resource_type, resource_id, meta FROM events.audit_log"
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(row.actor_id, Some(user.id));
            assert_eq!(row.source, "api");
            assert_eq!(row.action, "course.publish");
            assert_eq!(row.resource_type, "course");
            assert_eq!(row.resource_id, resource_id);
            assert_eq!(row.meta, json!({"role": "teacher"}));

            // A CLI row has no actor; the table refuses an api row without one and a cli row with one.
            let mut tx = pool.begin().await.unwrap();
            insert_audit(&mut tx, None, "cli", &entry()).await.unwrap();
            tx.commit().await.unwrap();
            for (actor, source) in [(None, "api"), (Some(user.id), "cli"), (None, "web")] {
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
            let row =
                sqlx::query!("SELECT type, version, payload, processed_at FROM events.outbox")
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
}
