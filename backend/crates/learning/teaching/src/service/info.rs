//! The header of a course, its lifecycle (archive, restore, delete), templates, the people on it
//! and the search for people to add (ADR-0048). What needs the content, the grades or the
//! attempts of the course is a flow above (`learning::courses`): the functions here do the part
//! that is `teaching`'s and say where the flow takes over.

use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use super::{NewCourse, TeachingError, insert_owned_course, load};
use crate::model::{
    Candidate, Course, CourseCode, CourseDescription, CourseHeader, CourseInfoUpdate, CourseKind,
    CoursePeople, CourseTitle, Person, TemplateRow,
};
use crate::repo;
use access::Actor;
use access::{self, Action, CourseAccess, CourseStatus};
use common::{Page, PageParams};
use events::{self, AuditEntry};

/// The most candidates a search returns.
const CANDIDATE_LIMIT: i64 = 20;
/// The shortest search text.
const CANDIDATE_MIN_CHARS: usize = 2;

fn course_entry(action: &'static str, course_id: Uuid, meta: serde_json::Value) -> AuditEntry {
    AuditEntry {
        action,
        resource_type: "course",
        resource_id: course_id,
        meta,
    }
}

/// The part of the header of a course that `teaching` knows, for anyone who may view it.
pub async fn course_header(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<CourseHeader, TeachingError> {
    load(conn, actor, course_id, false).await?;
    let row = repo::info_row(conn, course_id)
        .await?
        .ok_or(TeachingError::CourseNotFound)?;
    Ok(CourseHeader {
        course_id,
        code: row.code,
        description: row.description,
        term_name: row.term_name,
        owner_name: row.owner_name,
        owner_surname: row.owner_surname,
        owner_email: row.owner_email,
        student_count: row.student_count,
        assistant_count: row.assistant_count,
        updated_at: row.updated_at,
    })
}

/// Changes the code and/or the description. A field that is absent changes nothing; an empty one
/// clears it.
pub async fn update_course_info(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    update: CourseInfoUpdate,
) -> Result<(), TeachingError> {
    let (_, access) = load(conn, actor, course_id, true).await?;
    access::authorize(actor, Action::EditCourse(access))?;
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
        events::audit_implicit(
            conn,
            actor,
            access,
            course_entry("course.update", course_id, json!({"fields": fields})),
        )
        .await?;
    }
    Ok(())
}

/// Archives the course: it stays readable and takes no more writes. Archiving an archived course
/// changes nothing.
pub async fn archive_course(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<Course, TeachingError> {
    let (course, access) = load(conn, actor, course_id, true).await?;
    access::authorize(actor, Action::ArchiveCourse(access))?;
    if course.status == CourseStatus::Archived {
        return Ok(course);
    }
    let archived = repo::set_status(conn, course_id, CourseStatus::Archived).await?;
    events::audit_implicit(
        conn,
        actor,
        access,
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
    access::authorize(actor, Action::ArchiveCourse(access))?;
    if course.status != CourseStatus::Archived {
        return Ok(course);
    }
    let restored = repo::set_status(conn, course_id, CourseStatus::Published).await?;
    events::audit_implicit(
        conn,
        actor,
        access,
        course_entry("course.restore", course_id, json!({})),
    )
    .await?;
    Ok(restored)
}

/// First half of deleting a course: authorizes and locks the course row. The flow
/// (`learning::courses::delete`) then locks the assignment rows, asks content, grades and
/// attempts whether anything must be kept, purges the drafts, and calls [`finish_delete_course`]
/// (ADR-0035 point 12, ADR-0048).
pub async fn begin_delete_course(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<CourseAccess, TeachingError> {
    let (_, access) = load(conn, actor, course_id, true).await?;
    access::authorize(actor, Action::DeleteCourse(access))?;
    Ok(access)
}

/// Second half: audits and deletes the course, under the lock [`begin_delete_course`] took.
/// `access` is the one that call answered with.
pub async fn finish_delete_course(
    conn: &mut PgConnection,
    actor: &Actor,
    access: CourseAccess,
    course_id: Uuid,
) -> Result<(), TeachingError> {
    events::audit_implicit(
        conn,
        actor,
        access,
        course_entry("course.delete", course_id, json!({})),
    )
    .await?;
    if repo::delete_course(conn, course_id).await? == 0 {
        return Err(TeachingError::CourseNotFound);
    }
    Ok(())
}

/// The course row of a template made from `course_id`, owned by the actor. The flow
/// (`learning::courses::make_template`) copies the content into it.
pub async fn new_template(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    title: Option<CourseTitle>,
) -> Result<Course, TeachingError> {
    let (course, access) = load(conn, actor, course_id, false).await?;
    access::authorize(actor, Action::ManageContent(access))?;
    let (code, description) = repo::code_and_description(conn, course_id).await?;
    let title = match title {
        Some(title) => title,
        None => CourseTitle::parse(&course.title)?,
    };
    insert_owned_course(
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
    .await
}

/// The row of a draft course made from a template the actor can edit, owned by the actor. The
/// flow (`learning::courses::create_from_template`) copies the content into it.
pub async fn new_course_from_template(
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
    access::authorize(actor, Action::ManageContent(access))?;
    let (code, description) = repo::code_and_description(conn, template_id).await?;
    let title = match title {
        Some(title) => title,
        None => CourseTitle::parse(&template.title)?,
    };
    insert_owned_course(
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
    .await
}

/// The templates the actor owns or teaches. What each holds is content's to say.
pub async fn list_templates(
    conn: &mut PgConnection,
    actor: &Actor,
    page: PageParams,
) -> Result<Page<TemplateRow>, TeachingError> {
    access::authorize(actor, Action::ViewOwnCourses)?;
    let rows =
        repo::list_templates(conn, actor.user_id(), page.after, i64::from(page.limit) + 1).await?;
    Ok(Page::from_rows(rows, page.limit, |template| {
        template.course.id
    }))
}

/// Everyone on the roster of the course. Staff also get the emails and the linked groups, and the
/// flow adds the former students; a student's queries never read an email (ADR-0048).
pub async fn course_people(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
) -> Result<CoursePeople, TeachingError> {
    let (_, access) = load(conn, actor, course_id, false).await?;
    let staff = access::authorize(actor, Action::ManageRoster(access)).is_ok();
    let people = repo::list_people(conn, course_id, staff).await?;
    let groups = if staff {
        repo::list_link_summaries(conn, course_id).await?
    } else {
        Vec::new()
    };
    Ok(CoursePeople {
        people,
        groups,
        staff,
    })
}

/// Those of `candidates` (people who have a value or an attempt in the course) who are no longer
/// on its roster (ADR-0037), by surname. Emails only `with_emails`. The caller has authorized.
pub async fn former_people(
    conn: &mut PgConnection,
    course_id: Uuid,
    candidates: &[Uuid],
    with_emails: bool,
) -> Result<Vec<Person>, TeachingError> {
    Ok(repo::list_former(conn, course_id, candidates, with_emails).await?)
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
    access::authorize(actor, Action::ManageRoster(access))?;
    let q = q.trim();
    if q.chars().count() < CANDIDATE_MIN_CHARS {
        return Err(TeachingError::QueryTooShort);
    }
    Ok(repo::search_candidates(conn, course_id, &like_pattern(q), CANDIDATE_LIMIT).await?)
}

#[cfg(test)]
mod tests;
