//! SQL only. The sources of the facts: submissions of `assessment` and grades of `gradebook`.

use sqlx::PgConnection;
use sqlx::types::Decimal;

use crate::content::{FactScope, SourceFact};

/// What the sources say inside the scope: for each student and assignment the first submitted
/// attempt and the published score, one row where either exists. A draft attempt and a draft
/// grade are not selected.
pub async fn sources(
    conn: &mut PgConnection,
    scope: &FactScope,
) -> Result<Vec<SourceFact>, sqlx::Error> {
    let (item, student, course) = scope.filters();
    let rows = sqlx::query!(
        r#"SELECT item_id AS "item_id!", student_id AS "student_id!",
                  min(submitted_at) AS submitted_at,
                  max(published_score) AS "published_score: Decimal"
           FROM (
               SELECT s.assignment_id AS item_id, s.student_id, s.submitted_at,
                      NULL::numeric AS published_score
               FROM assessment.submission_index s
               WHERE s.status = 'submitted'
               UNION ALL
               SELECT g.assignment_id, g.student_id, NULL::timestamptz, g.published_score
               FROM gradebook.published_grades g
               WHERE g.published_score IS NOT NULL
           ) found
           WHERE ($1::uuid IS NULL OR item_id = $1) AND ($2::uuid IS NULL OR student_id = $2)
             AND ($3::uuid IS NULL
                  OR item_id IN (SELECT id FROM content.item_list WHERE course_id = $3))
           GROUP BY item_id, student_id"#,
        item,
        student,
        course
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| SourceFact {
            item_id: row.item_id,
            student_id: row.student_id,
            submitted_at: row.submitted_at,
            published_score: row.published_score,
        })
        .collect())
}
