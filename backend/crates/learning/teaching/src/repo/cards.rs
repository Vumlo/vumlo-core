//! The read model behind the course cards, the work list and search (ADR-0052): the courses of one
//! person with what a card shows of the course itself.

use sqlx::PgConnection;
use uuid::Uuid;

use super::{CourseRow, RawCourseRow};
use crate::model::{CardCourse, CourseKind};
use access::{CourseRole, CourseStatus};

/// The regular courses of the user, with what a card shows of the course itself. A student sees
/// a course only while it is published or archived (ADR-0048).
pub async fn card_courses(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<Vec<CardCourse>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT c.id, c.kind AS "kind: CourseKind", c.title, c.status AS "status: CourseStatus",
                  c.owner_id, c.term_id, c.created_at, c.code, c.description,
                  t.name AS "term_name?", t.ends_on AS "term_ends?",
                  o.name AS "owner_name!", o.surname AS "owner_surname!",
                  m.role AS "member_role?: CourseRole",
                  (SELECT count(*) FROM teaching.effective_members x
                   WHERE x.course_id = c.id AND x.role = 'student') AS "student_count!",
                  (SELECT count(*) FROM teaching.effective_members x
                   WHERE x.course_id = c.id AND x.role = 'assistant') AS "assistant_count!"
           FROM (SELECT course_id AS id FROM teaching.effective_members WHERE user_id = $1
                 UNION
                 SELECT id FROM teaching.courses WHERE owner_id = $1) mine
           JOIN teaching.courses c ON c.id = mine.id
           LEFT JOIN teaching.effective_members m ON m.course_id = c.id AND m.user_id = $1
           LEFT JOIN org.term_list t ON t.id = c.term_id
           JOIN auth.people o ON o.id = c.owner_id
           WHERE c.kind = 'regular'
             AND (c.status IN ('published', 'archived') OR c.owner_id = $1
                  OR m.role IN ('teacher', 'assistant'))
           ORDER BY c.title, c.id"#,
        user_id
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let raw = RawCourseRow {
                id: row.id,
                kind: row.kind,
                title: row.title,
                status: row.status,
                owner_id: row.owner_id,
                term_id: row.term_id,
                created_at: row.created_at,
                member_role: row.member_role,
            };
            CardCourse {
                course: CourseRow::from(raw).course,
                code: row.code,
                description: row.description,
                term_name: row.term_name,
                term_ends: row.term_ends,
                owner_name: row.owner_name,
                owner_surname: row.owner_surname,
                role: row.member_role,
                student_count: row.student_count,
                assistant_count: row.assistant_count,
            }
        })
        .collect())
}
