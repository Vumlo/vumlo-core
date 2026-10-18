//! The course header, archive, deletion, templates, people and candidates (ADR-0048).

use api_types::{
    Candidate, Candidates, CandidatesQuery, Course, CourseInfo, CoursePeople,
    CreateFromTemplateRequest, LinkedGroup, MakeTemplateRequest, Page, PageQuery, PersonRow,
    ProblemDetails, TemplateSummary, UpdateCourseInfoRequest,
};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use sqlx::types::Decimal;
use uuid::Uuid;

use super::{api_course, api_role, api_source, page_params};
use crate::actor::Actor;
use crate::app::AppState;
use crate::error::{ApiError, ValidJson, ValidQuery};
use crate::identity::Role;
use crate::teaching::model::{self, CourseInfoUpdate, CourseTitle};
use crate::teaching::service::{self, TeachingError};

/// Counts are far below `u32::MAX`; a database that disagreed would still not overflow the field.
fn count(n: i64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Points always have two decimals (ADR-0021).
fn points(value: Decimal) -> String {
    format!("{value:.2}")
}

fn api_info(info: model::CourseInfo) -> CourseInfo {
    CourseInfo {
        course_id: info.course_id,
        code: info.code,
        description: info.description,
        term_name: info.term_name,
        owner_name: info.owner_name,
        owner_surname: info.owner_surname,
        owner_email: info.owner_email,
        student_count: count(info.student_count),
        former_count: count(info.former_count),
        assistant_count: count(info.assistant_count),
        updated_at: info.updated_at,
        content_updated_at: info.content_updated_at,
        total_points: points(info.total_points),
        graded_items: count(info.graded_items),
        has_grades: info.has_grades,
    }
}

fn api_person(person: model::Person) -> PersonRow {
    PersonRow {
        user_id: person.user_id,
        name: person.name,
        surname: person.surname,
        email: person.email,
        role: api_role(person.role),
        is_owner: person.is_owner,
        source: api_source(person.source),
        group: person.group,
        former: person.former,
    }
}

fn api_linked_group(group: model::LinkedGroup) -> LinkedGroup {
    LinkedGroup {
        link_id: group.link_id,
        group_name: group.group_name,
        subgroup_name: group.subgroup_name,
        student_count: count(group.student_count),
        linked_at: group.linked_at,
    }
}

fn api_candidate(candidate: model::Candidate) -> Candidate {
    Candidate {
        user_id: candidate.user_id,
        name: candidate.name,
        surname: candidate.surname,
        email: candidate.email,
        role: match candidate.role {
            Role::Admin => api_types::Role::Admin,
            Role::Teacher => api_types::Role::Teacher,
            Role::Student => api_types::Role::Student,
        },
    }
}

/// The header of a course: code, description, term, owner, counts and the points it adds up to.
/// Anyone who may view the course.
#[utoipa::path(
    get,
    path = "/courses/{id}/info",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    responses(
        (status = 200, description = "The course header", body = CourseInfo),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn get_course_info(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<CourseInfo>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(TeachingError::from)?;
    let info = service::course_info(&mut conn, &actor, id).await?;
    Ok(Json(api_info(info)))
}

/// Changes the code and/or the description of a course. Assistants, teachers, owners and admins;
/// not in an archived course. Only the fields present change; an empty one clears the field.
#[utoipa::path(
    patch,
    path = "/courses/{id}/info",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    request_body = UpdateCourseInfoRequest,
    responses(
        (status = 200, description = "The course header after the change", body = CourseInfo),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not edit this course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The course is archived", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "A code or a description that is too long",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn update_course_info(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<UpdateCourseInfoRequest>,
) -> Result<Json<CourseInfo>, ApiError> {
    let update = CourseInfoUpdate {
        code: request.code,
        description: request.description,
    };
    let mut tx = state.db.begin().await.map_err(TeachingError::from)?;
    let info = service::update_course_info(&mut tx, &actor, id, update).await?;
    tx.commit().await.map_err(TeachingError::from)?;
    Ok(Json(api_info(info)))
}

/// Archives a course: it stays readable, for students too, and takes no more changes. Owner,
/// course teacher or admin. Idempotent.
#[utoipa::path(
    post,
    path = "/courses/{id}/archive",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    responses(
        (status = 200, description = "The archived course", body = Course),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not archive this course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn archive_course(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<Course>, ApiError> {
    let mut tx = state.db.begin().await.map_err(TeachingError::from)?;
    let course = service::archive_course(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(TeachingError::from)?;
    Ok(Json(api_course(course)))
}

/// Takes a course out of the archive. It is published again, also when it was a draft before.
/// Owner, course teacher or admin. Idempotent.
#[utoipa::path(
    post,
    path = "/courses/{id}/restore",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    responses(
        (status = 200, description = "The restored course", body = Course),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not restore this course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn restore_course(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<Course>, ApiError> {
    let mut tx = state.db.begin().await.map_err(TeachingError::from)?;
    let course = service::restore_course(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(TeachingError::from)?;
    Ok(Json(api_course(course)))
}

/// Deletes a course with its content, members and links. The owner or an admin, in every status.
///
/// A course in which any grade has a value or any work was submitted cannot be deleted: archive
/// it instead.
#[utoipa::path(
    delete,
    path = "/courses/{id}",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "Only the owner or an admin may delete a course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The course has grades or submitted work",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn delete_course(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db.begin().await.map_err(TeachingError::from)?;
    service::delete_course(&mut tx, &actor, id).await?;
    tx.commit().await.map_err(TeachingError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The templates the caller owns or teaches, with how many sections, lectures and assignments
/// each holds.
#[utoipa::path(
    get,
    path = "/me/templates",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "A page of templates", body = Page<TemplateSummary>),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "Invalid cursor", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn list_templates(
    State(state): State<AppState>,
    actor: Actor,
    ValidQuery(query): ValidQuery<PageQuery>,
) -> Result<Json<Page<TemplateSummary>>, ApiError> {
    let page = page_params(&query)?;
    let mut conn = state.db.acquire().await.map_err(TeachingError::from)?;
    let templates = service::list_templates(&mut conn, &actor, page).await?;
    Ok(Json(Page {
        items: templates
            .items
            .into_iter()
            .map(|template| TemplateSummary {
                course: api_course(template.course),
                sections: count(template.sections),
                lectures: count(template.lectures),
                assignments: count(template.assignments),
                updated_at: template.updated_at,
            })
            .collect(),
        next_cursor: templates.next.map(|id| id.to_string()),
    }))
}

/// Makes a template from a course: a copy of its whole tree, as drafts, owned by the caller.
///
/// Anyone who can edit the course's content and may create courses. Members, groups, deadlines,
/// submissions and grades are not copied. Works on an archived course.
#[utoipa::path(
    post,
    path = "/courses/{id}/template",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    request_body = MakeTemplateRequest,
    responses(
        (status = 201, description = "The new template", body = Course),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not do this", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid title", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn make_template(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<MakeTemplateRequest>,
) -> Result<(StatusCode, Json<Course>), ApiError> {
    let title = request
        .title
        .as_deref()
        .map(CourseTitle::parse)
        .transpose()
        .map_err(TeachingError::from)?;
    let mut tx = state.db.begin().await.map_err(TeachingError::from)?;
    let template = service::make_template(&mut tx, &actor, id, title).await?;
    tx.commit().await.map_err(TeachingError::from)?;
    Ok((StatusCode::CREATED, Json(api_course(template))))
}

/// Makes a draft course from a template: a copy of its whole tree, as drafts, owned by the caller.
#[utoipa::path(
    post,
    path = "/templates/{id}/courses",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Template id")),
    request_body = CreateFromTemplateRequest,
    responses(
        (status = 201, description = "The new course", body = Course),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not do this", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such template, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid title or an unknown term", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn create_from_template(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<CreateFromTemplateRequest>,
) -> Result<(StatusCode, Json<Course>), ApiError> {
    let title = request
        .title
        .as_deref()
        .map(CourseTitle::parse)
        .transpose()
        .map_err(TeachingError::from)?;
    let mut tx = state.db.begin().await.map_err(TeachingError::from)?;
    let course = service::create_from_template(&mut tx, &actor, id, title, request.term_id).await?;
    tx.commit().await.map_err(TeachingError::from)?;
    Ok((StatusCode::CREATED, Json(api_course(course))))
}

/// The people on a course with their names. Anyone who may view the course.
///
/// Staff also get the emails, the former students (ADR-0037) and the linked groups; a student gets
/// names and groups of classmates and no address.
#[utoipa::path(
    get,
    path = "/courses/{id}/people",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    responses(
        (status = 200, description = "The people", body = CoursePeople),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn get_people(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<CoursePeople>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(TeachingError::from)?;
    let people = service::course_people(&mut conn, &actor, id).await?;
    Ok(Json(CoursePeople {
        people: people.people.into_iter().map(api_person).collect(),
        groups: people.groups.into_iter().map(api_linked_group).collect(),
    }))
}

/// Active people who are not on the course and match `q` in their name, surname or email: at
/// most 20, ordered by surname. Whoever manages the roster (assistants included).
#[utoipa::path(
    get,
    path = "/courses/{id}/candidates",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id"), CandidatesQuery),
    responses(
        (status = 200, description = "Up to 20 people", body = Candidates),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 403, description = "The caller may not manage this roster",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A search text shorter than two characters",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub async fn get_candidates(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<Uuid>,
    ValidQuery(query): ValidQuery<CandidatesQuery>,
) -> Result<Json<Candidates>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(TeachingError::from)?;
    let found = service::course_candidates(&mut conn, &actor, id, &query.q).await?;
    Ok(Json(Candidates {
        items: found.into_iter().map(api_candidate).collect(),
    }))
}
