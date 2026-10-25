//! SQL only. Every function takes the connection it runs on and never begins a transaction.

use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{AuditFilter, AuditRow};

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
               FROM events.audit_entries l
               LEFT JOIN auth.people a ON a.id = l.actor_id
               LEFT JOIN auth.people ru ON l.resource_type = 'user' AND ru.id = l.resource_id
               LEFT JOIN teaching.courses rc ON l.resource_type = 'course' AND rc.id = l.resource_id
               LEFT JOIN org.group_list rg ON l.resource_type = 'group' AND rg.id = l.resource_id
               LEFT JOIN org.subgroup_list rs ON l.resource_type = 'subgroup' AND rs.id = l.resource_id
               LEFT JOIN org.term_list rt ON l.resource_type = 'term' AND rt.id = l.resource_id
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
