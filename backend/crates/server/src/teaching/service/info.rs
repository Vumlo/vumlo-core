//! The header of a course, its lifecycle (archive, restore, delete), templates, the people on it
//! and the search for people to add (ADR-0048).

use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use super::{NewCourse, TeachingError, insert_owned_course, load};
use crate::actor::Actor;
use crate::assessment;
use crate::content;
use crate::events::{self, AuditEntry};
use crate::gradebook;
use crate::pagination::{Page, PageParams};
use crate::policy::{self, Action, CourseStatus};
use crate::teaching::model::{
    Candidate, Course, CourseCode, CourseDescription, CourseInfo, CourseInfoUpdate, CourseKind,
    CoursePeople, CourseTitle, TemplateSummary,
};
use crate::teaching::repo;

/// The most candidates a search returns.
const CANDIDATE_LIMIT: i64 = 20;
/// The shortest search text.
const CANDIDATE_MIN_CHARS: usize = 2;

fn from_content(error: content::ContentError) -> TeachingError {
    TeachingError::Content(Box::new(error))
}

fn from_gradebook(error: gradebook::GradebookError) -> TeachingError {
    TeachingError::Gradebook(Box::new(error))
}

fn from_assessment(error: assessment::AssessmentError) -> TeachingError {
    TeachingError::Assessment(Box::new(error))
}

fn course_entry(action: &'static str, course_id: Uuid, meta: serde_json::Value) -> AuditEntry {
    AuditEntry {
        action,
        resource_type: "course",
        resource_id: course_id,
        meta,
    }
}

/// The header of a course for anyone who may view it.
pub async fn course_info(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<CourseInfo, TeachingError> {
    load(conn, actor, course_id, false).await?;
    let row = repo::info_row(conn, course_id)
        .await?
        .ok_or(TeachingError::CourseNotFound)?;
    let summary = content::course_summary(conn, actor, course_id)
        .await
        .map_err(from_content)?;
    let has_grades = gradebook::has_grades_in_course(conn, course_id)
        .await
        .map_err(from_gradebook)?;
    Ok(CourseInfo {
        course_id,
        code: row.code,
        description: row.description,
        term_name: row.term_name,
        owner_name: row.owner_name,
        owner_surname: row.owner_surname,
        owner_email: row.owner_email,
        student_count: row.student_count,
        former_count: row.former_count,
        assistant_count: row.assistant_count,
        updated_at: row.updated_at,
        content_updated_at: summary.changed_at,
        total_points: summary.total_points,
        graded_items: summary.graded_items,
        has_grades,
    })
}

/// Changes the code and/or the description. A field that is absent changes nothing; an empty one
/// clears it.
pub async fn update_course_info(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    update: CourseInfoUpdate,
) -> Result<CourseInfo, TeachingError> {
    let (_, access) = load(conn, actor, course_id, true).await?;
    policy::authorize(actor, Action::EditCourse(access))?;
    let code = update.code.as_deref().map(CourseCode::parse).transpose()?;
    let description = update
        .description
        .as_deref()
        .map(CourseDescription::parse)
        .transpose()?;
    if code.is_some() || description.is_some() {
        let mut fields = Vec::new();
        if code.is_some() {
            fields.push("code");
        }
        if description.is_some() {
            fields.push("description");
        }
        repo::update_info(
            conn,
            course_id,
            code.as_ref().map(CourseCode::as_str),
            description.as_ref().map(CourseDescription::as_str),
        )
        .await?;
        events::audit_admin(
            conn,
            actor,
            course_entry("course.update", course_id, json!({"fields": fields})),
        )
        .await?;
    }
    course_info(conn, actor, course_id).await
}

/// Archives the course: it stays readable and takes no more writes. Archiving an archived course
/// changes nothing.
pub async fn archive_course(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<Course, TeachingError> {
    let (course, access) = load(conn, actor, course_id, true).await?;
    policy::authorize(actor, Action::ArchiveCourse(access))?;
    if course.status == CourseStatus::Archived {
        return Ok(course);
    }
    let archived = repo::set_status(conn, course_id, CourseStatus::Archived).await?;
    events::audit_admin(
        conn,
        actor,
        course_entry("course.archive", course_id, json!({})),
    )
    .await?;
    Ok(archived)
}

/// Takes the course out of the archive; it is published again. Restoring a course that is not
/// archived changes nothing.
pub async fn restore_course(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<Course, TeachingError> {
    let (course, access) = load(conn, actor, course_id, true).await?;
    policy::authorize(actor, Action::ArchiveCourse(access))?;
    if course.status != CourseStatus::Archived {
        return Ok(course);
    }
    let restored = repo::set_status(conn, course_id, CourseStatus::Published).await?;
    events::audit_admin(
        conn,
        actor,
        course_entry("course.restore", course_id, json!({})),
    )
    .await?;
    Ok(restored)
}

/// Deletes a course nobody has graded or submitted work in. The assignment rows are locked first
/// and the guards ask in later statements, so a grade write or a submit in flight is seen
/// (ADR-0035 point 12, ADR-0048).
pub async fn delete_course(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<(), TeachingError> {
    let (_, access) = load(conn, actor, course_id, true).await?;
    policy::authorize(actor, Action::DeleteCourse(access))?;
    content::lock_course_assignments(conn, course_id)
        .await
        .map_err(from_content)?;
    if gradebook::has_grades_in_course(conn, course_id)
        .await
        .map_err(from_gradebook)?
    {
        return Err(TeachingError::HasGrades);
    }
    if assessment::has_submitted_in_course(conn, course_id)
        .await
        .map_err(from_assessment)?
    {
        return Err(TeachingError::HasSubmissions);
    }
    gradebook::purge_course(conn, course_id)
        .await
        .map_err(from_gradebook)?;
    assessment::purge_course(conn, course_id)
        .await
        .map_err(from_assessment)?;
    events::audit_admin(
        conn,
        actor,
        course_entry("course.delete", course_id, json!({})),
    )
    .await?;
    if repo::delete_course(conn, course_id).await? == 0 {
        return Err(TeachingError::CourseNotFound);
    }
    Ok(())
}

/// A copy of the course for the next time: a template owned by the actor with the course's whole
/// tree as drafts.
pub async fn make_template(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    title: Option<CourseTitle>,
) -> Result<Course, TeachingError> {
    let (course, access) = load(conn, actor, course_id, false).await?;
    policy::authorize(actor, Action::ManageContent(access))?;
    let (code, description) = repo::code_and_description(conn, course_id).await?;
    let title = match title {
        Some(title) => title,
        None => CourseTitle::parse(&course.title)?,
    };
    let template = insert_owned_course(
        conn,
        actor,
        NewCourse {
            kind: CourseKind::Template,
            title: &title,
            term_id: None,
            code: &code,
            description: &description,
            from: Some(course_id),
        },
    )
    .await?;
    content::copy_course_content(conn, course_id, template.id)
        .await
        .map_err(from_content)?;
    Ok(template)
}

/// A draft course owned by the actor with the whole tree of a template they can edit.
pub async fn create_from_template(
    conn: &mut PgConnection,
    actor: &Actor,
    template_id: Uuid,
    title: Option<CourseTitle>,
    term_id: Option<Uuid>,
) -> Result<Course, TeachingError> {
    let (template, access) = load(conn, actor, template_id, false).await?;
    if template.kind != CourseKind::Template {
        return Err(TeachingError::CourseNotFound);
    }
    policy::authorize(actor, Action::ManageContent(access))?;
    let (code, description) = repo::code_and_description(conn, template_id).await?;
    let title = match title {
        Some(title) => title,
        None => CourseTitle::parse(&template.title)?,
    };
    let course = insert_owned_course(
        conn,
        actor,
        NewCourse {
            kind: CourseKind::Regular,
            title: &title,
            term_id,
            code: &code,
            description: &description,
            from: Some(template_id),
        },
    )
    .await?;
    content::copy_course_content(conn, template_id, course.id)
        .await
        .map_err(from_content)?;
    Ok(course)
}

/// The templates the actor owns or teaches, with what each holds.
pub async fn list_templates(
    conn: &mut PgConnection,
    actor: &Actor,
    page: PageParams,
) -> Result<Page<TemplateSummary>, TeachingError> {
    policy::authorize(actor, Action::ViewOwnCourses)?;
    let rows =
        repo::list_templates(conn, actor.user_id(), page.after, i64::from(page.limit) + 1).await?;
    Ok(Page::from_rows(rows, page.limit, |template| {
        template.course.id
    }))
}

/// Everyone on the course. Staff also get the emails, the former students and the linked groups;
/// a student's queries never read an email (ADR-0048).
pub async fn course_people(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<CoursePeople, TeachingError> {
    let (_, access) = load(conn, actor, course_id, false).await?;
    let staff = policy::authorize(actor, Action::ManageRoster(access)).is_ok();
    let mut people = repo::list_people(conn, course_id, staff).await?;
    let mut groups = Vec::new();
    if staff {
        people.extend(repo::list_former(conn, course_id).await?);
        groups = repo::list_link_summaries(conn, course_id).await?;
    }
    Ok(CoursePeople { people, groups })
}

/// Escapes `\`, `%` and `_` so that the text is matched as it is.
fn like_pattern(text: &str) -> String {
    let mut pattern = String::with_capacity(text.len() + 2);
    pattern.push('%');
    for c in text.chars() {
        if matches!(c, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    pattern
}

/// Active people who are not on the course and whose name, surname or email contains `q`.
pub async fn course_candidates(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    q: &str,
) -> Result<Vec<Candidate>, TeachingError> {
    let (_, access) = load(conn, actor, course_id, false).await?;
    policy::authorize(actor, Action::ManageRoster(access))?;
    let q = q.trim();
    if q.chars().count() < CANDIDATE_MIN_CHARS {
        return Err(TeachingError::QueryTooShort);
    }
    Ok(repo::search_candidates(conn, course_id, &like_pattern(q), CANDIDATE_LIMIT).await?)
}

#[cfg(test)]
mod tests;
