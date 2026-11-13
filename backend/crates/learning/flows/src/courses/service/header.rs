//! What a course page shows about the course: its header, its people and the templates of the
//! actor. The part `teaching` knows is read first, then `content`, `gradebook` and `assessment`
//! are asked about the same course (ADR-0093).

use std::collections::HashMap;

use sqlx::PgConnection;
use uuid::Uuid;

use crate::LearningError;
use crate::courses::model::{CourseInfo, Template};
use crate::{assessment, gradebook};
use access::Actor;
use common::{Page, PageParams};
use teaching::{self, CourseInfoUpdate, CoursePeople, Person};

/// The people off the roster of the course who still have a value or an attempt in it
/// (ADR-0037): `gradebook` and `assessment` say who has one, `teaching` who is no longer on the
/// roster. The caller has authorized.
async fn former(
    conn: &mut PgConnection,
    course_id: Uuid,
    with_emails: bool,
) -> Result<Vec<Person>, LearningError> {
    let mut who = gradebook::students_with_values(conn, course_id).await?;
    who.extend(assessment::students_with_attempts(conn, course_id).await?);
    Ok(teaching::former_people(conn, course_id, &who, with_emails).await?)
}

/// The header of a course for anyone who may view it.
pub async fn info(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<CourseInfo, LearningError> {
    let header = teaching::course_header(conn, actor, course_id).await?;
    let summary = content::course_summary(conn, actor, course_id).await?;
    let has_grades = gradebook::has_grades_in_course(conn, course_id).await?;
    let former_count = former(conn, course_id, false).await?.len();
    Ok(CourseInfo {
        course_id,
        code: header.code,
        description: header.description,
        term_name: header.term_name,
        owner_name: header.owner_name,
        owner_surname: header.owner_surname,
        owner_email: header.owner_email,
        student_count: header.student_count,
        former_count: i64::try_from(former_count).unwrap_or(i64::MAX),
        assistant_count: header.assistant_count,
        updated_at: header.updated_at,
        content_updated_at: summary.changed_at,
        total_points: summary.total_points,
        graded_items: summary.graded_items,
        has_grades,
    })
}

/// Changes the code and/or the description, and returns the header after the change.
pub async fn update_info(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    update: CourseInfoUpdate,
) -> Result<CourseInfo, LearningError> {
    teaching::update_course_info(conn, actor, course_id, update).await?;
    info(conn, actor, course_id).await
}

/// Everyone on the course. Staff also get the emails, the former students and the linked groups;
/// a student's queries never read an email (ADR-0048).
pub async fn people(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<CoursePeople, LearningError> {
    let mut people = teaching::course_people(conn, actor, course_id).await?;
    if people.staff {
        people.people.extend(former(conn, course_id, true).await?);
    }
    Ok(people)
}

/// The templates the actor owns or teaches, with what each holds.
pub async fn templates(
    conn: &mut PgConnection,
    actor: &Actor,
    page: PageParams,
) -> Result<Page<Template>, LearningError> {
    let rows = teaching::list_templates(conn, actor, page).await?;
    let ids: Vec<Uuid> = rows.items.iter().map(|row| row.course.id).collect();
    let mut counts: HashMap<Uuid, content::TemplateCounts> = content::template_counts(conn, &ids)
        .await?
        .into_iter()
        .map(|counts| (counts.course_id, counts))
        .collect();
    let items = rows
        .items
        .into_iter()
        .map(|row| {
            let held = counts.remove(&row.course.id);
            Template {
                sections: held.as_ref().map_or(0, |held| held.sections),
                lectures: held.as_ref().map_or(0, |held| held.lectures),
                assignments: held.as_ref().map_or(0, |held| held.assignments),
                course: row.course,
                updated_at: row.updated_at,
            }
        })
        .collect();
    Ok(Page {
        items,
        next: rows.next,
    })
}
