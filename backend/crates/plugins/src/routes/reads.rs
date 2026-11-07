//! The read surface of the plugin API (ADR-0074). Every handler takes the plugin's token, opens
//! no transaction, and asks `learning::plugin_read`, which checks the scope and the course. Every
//! answer is `no-store`: it may name students.

use access::{PluginCaller, PluginSource};
use api_types::{
    MAX_CONTACT_IDS, Page, PluginAssignment, PluginAssignmentsQuery, PluginAttempt, PluginContact,
    PluginContactQuery, PluginContacts, PluginFormAnswers, PluginGrade, PluginGradesQuery,
    PluginListQuery, PluginMember, PluginMembersQuery, PluginQuiz, PluginSubmission,
    PluginSubmissionItem, ProblemDetails,
};
use axum::Json;
use axum::extract::{FromRef, Path, State};
use axum::http::{HeaderName, HeaderValue};
use common::{ApiError, ValidQuery};
use learning::plugin_read::{self, PluginList, PluginPage};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use super::{db, no_store};
use crate::state::PluginsState;

pub fn router<S>() -> OpenApiRouter<S>
where
    S: PluginSource + Clone + 'static,
    PluginsState: FromRef<S>,
{
    OpenApiRouter::new()
        .routes(routes!(list_assignments))
        .routes(routes!(get_assignment))
        .routes(routes!(get_form_answers))
        .routes(routes!(get_submission))
        .routes(routes!(list_submissions))
        .routes(routes!(list_members))
        .routes(routes!(list_grades))
        .routes(routes!(get_quiz))
        .routes(routes!(list_attempts))
        .routes(routes!(get_contacts))
}

type Reply<T> = Result<([(HeaderName, HeaderValue); 1], Json<T>), ApiError>;

fn reply<T>(body: T) -> Reply<T> {
    Ok(([no_store()], Json(body)))
}

fn page<T, U>(list: PluginList<T>, map: impl Fn(T) -> U) -> Page<U> {
    Page {
        items: list.items.into_iter().map(map).collect(),
        next_cursor: list.next.map(|cursor| cursor.to_string()),
    }
}

/// The assignments a student could see: open, with their published titles. The scope
/// `assignments:read`. Without `course_id`, those of every course the plugin was granted. Sorted by
/// the last change; `updated_since` is the time of that change.
#[utoipa::path(
    get,
    path = "/plugin-api/assignments",
    tag = "plugin-api",
    security(("plugin" = [])),
    params(PluginAssignmentsQuery),
    responses(
        (status = 200, description = "A page of assignments", body = Page<PluginAssignment>),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `assignments:read`, or not the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "`course_id` names no course", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "A cursor or a time that is not valid", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn list_assignments(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    ValidQuery(query): ValidQuery<PluginAssignmentsQuery>,
) -> Reply<Page<PluginAssignment>> {
    let page_params = PluginPage::new(query.cursor.as_deref(), query.limit)?;
    let mut conn = state.db.acquire().await.map_err(db)?;
    let list = plugin_read::assignments(
        &mut conn,
        &caller,
        query.course_id,
        query.updated_since,
        page_params,
    )
    .await?;
    reply(page(list, |a| PluginAssignment {
        id: a.id,
        course_id: a.course_id,
        kind: a.kind,
        title: a.title,
        due_at: a.due_at,
        max_points: a.max_points,
        fields: a.fields,
        updated_at: a.updated_at,
    }))
}

/// One assignment a student could see. The scope `assignments:read`.
#[utoipa::path(
    get,
    path = "/plugin-api/assignments/{id}",
    tag = "plugin-api",
    security(("plugin" = [])),
    params(("id" = Uuid, Path, description = "The `assignment_id` of an event")),
    responses(
        (status = 200, description = "The assignment", body = PluginAssignment),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `assignments:read`, or not the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such assignment, or none a student could see",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn get_assignment(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    Path(id): Path<Uuid>,
) -> Reply<PluginAssignment> {
    let mut conn = state.db.acquire().await.map_err(db)?;
    let a = plugin_read::assignment(&mut conn, &caller, id).await?;
    reply(PluginAssignment {
        id: a.id,
        course_id: a.course_id,
        kind: a.kind,
        title: a.title,
        due_at: a.due_at,
        max_points: a.max_points,
        fields: a.fields,
        updated_at: a.updated_at,
    })
}

/// The values a student gave in a submitted `form`, by field id. The scope `assignments:read`.
#[utoipa::path(
    get,
    path = "/plugin-api/submissions/{id}/fields",
    tag = "plugin-api",
    security(("plugin" = [])),
    params(("id" = Uuid, Path, description = "The `submission_id` of an event")),
    responses(
        (status = 200, description = "The values", body = PluginFormAnswers),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `assignments:read`, or not the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such submitted attempt, or not at a form", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn get_form_answers(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    Path(id): Path<Uuid>,
) -> Reply<PluginFormAnswers> {
    let mut conn = state.db.acquire().await.map_err(db)?;
    let a = plugin_read::form_answers(&mut conn, &caller, id).await?;
    reply(PluginFormAnswers {
        submission_id: a.submission_id,
        course_id: a.course_id,
        assignment_id: a.assignment_id,
        student_id: a.student_id,
        answers: a.answers,
    })
}

/// A submitted attempt: which work, whose, and what it says. The scope `submissions:read`. Never a
/// draft, a score or a grade.
#[utoipa::path(
    get,
    path = "/plugin-api/submissions/{id}",
    tag = "plugin-api",
    security(("plugin" = [])),
    params(("id" = Uuid, Path, description = "The `submission_id` of the event")),
    responses(
        (status = 200, description = "The submitted attempt", body = PluginSubmission),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `submissions:read`, or not the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such submitted attempt", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn get_submission(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    Path(id): Path<Uuid>,
) -> Reply<PluginSubmission> {
    let mut conn = state.db.acquire().await.map_err(db)?;
    let work = plugin_read::submission(&mut conn, &caller, id).await?;
    reply(PluginSubmission {
        id: work.id,
        course_id: work.course_id,
        assignment_id: work.assignment_id,
        student_id: work.student_id,
        attempt: work.attempt,
        content: work.content,
        submitted_at: work.submitted_at,
    })
}

/// The submitted attempts of an assignment, without their content, by the time they were
/// submitted; `updated_since` is that time. The scope `submissions:read`.
#[utoipa::path(
    get,
    path = "/plugin-api/assignments/{id}/submissions",
    tag = "plugin-api",
    security(("plugin" = [])),
    params(("id" = Uuid, Path, description = "An assignment"), PluginListQuery),
    responses(
        (status = 200, description = "A page of attempts", body = Page<PluginSubmissionItem>),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `submissions:read`, or not the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such assignment", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "A cursor or a time that is not valid", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn list_submissions(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    Path(id): Path<Uuid>,
    ValidQuery(query): ValidQuery<PluginListQuery>,
) -> Reply<Page<PluginSubmissionItem>> {
    let page_params = PluginPage::new(query.cursor.as_deref(), query.limit)?;
    let mut conn = state.db.acquire().await.map_err(db)?;
    let list =
        plugin_read::submissions(&mut conn, &caller, id, query.updated_since, page_params).await?;
    reply(page(list, |s| PluginSubmissionItem {
        id: s.id,
        student_id: s.student_id,
        attempt: s.attempt,
        submitted_at: s.submitted_at,
        is_late: s.is_late,
        graded_at: s.graded_at,
    }))
}

/// The people of a course: ids, names and roles, no contact data. The scope `roster:read`. A full
/// snapshot sorted by user id: the roster has no change time, so there is no `updated_since`; the
/// `enrollment.changed` event says when to read it again.
#[utoipa::path(
    get,
    path = "/plugin-api/courses/{id}/members",
    tag = "plugin-api",
    security(("plugin" = [])),
    params(("id" = Uuid, Path, description = "A course"), PluginMembersQuery),
    responses(
        (status = 200, description = "A page of people", body = Page<PluginMember>),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `roster:read`, or not the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such course", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "A cursor that is not valid", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn list_members(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    Path(id): Path<Uuid>,
    ValidQuery(query): ValidQuery<PluginMembersQuery>,
) -> Reply<Page<PluginMember>> {
    let page_params = PluginPage::new(query.cursor.as_deref(), query.limit)?;
    let role = query.role.map(|role| match role {
        api_types::CourseRole::Teacher => access::CourseRole::Teacher,
        api_types::CourseRole::Assistant => access::CourseRole::Assistant,
        api_types::CourseRole::Student => access::CourseRole::Student,
    });
    let mut conn = state.db.acquire().await.map_err(db)?;
    let list = plugin_read::members(&mut conn, &caller, id, role, page_params).await?;
    reply(page(list, |m| PluginMember {
        user_id: m.user_id,
        name: m.name,
        surname: m.surname,
        role: match m.role.as_str() {
            "teacher" => api_types::CourseRole::Teacher,
            "assistant" => api_types::CourseRole::Assistant,
            _ => api_types::CourseRole::Student,
        },
        source: m.source,
    }))
}

/// Published grades, never a draft, by the time they were published; `updated_since` is that time.
/// The scope `grades:read`. Without `course_id`, those of every course the plugin was granted.
#[utoipa::path(
    get,
    path = "/plugin-api/grades",
    tag = "plugin-api",
    security(("plugin" = [])),
    params(PluginGradesQuery),
    responses(
        (status = 200, description = "A page of grades", body = Page<PluginGrade>),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `grades:read`, or not the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "`course_id` or `assignment_id` names nothing",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A cursor or a time that is not valid", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn list_grades(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    ValidQuery(query): ValidQuery<PluginGradesQuery>,
) -> Reply<Page<PluginGrade>> {
    let page_params = PluginPage::new(query.cursor.as_deref(), query.limit)?;
    let mut conn = state.db.acquire().await.map_err(db)?;
    let list = plugin_read::grades(
        &mut conn,
        &caller,
        query.course_id,
        query.assignment_id,
        query.updated_since,
        page_params,
    )
    .await?;
    reply(page(list, |g| PluginGrade {
        course_id: g.course_id,
        assignment_id: g.assignment_id,
        student_id: g.student_id,
        score: g.score,
        max_points: g.max_points,
        published_by: g.published_by,
        published_at: g.published_at,
    }))
}

/// The questions of a quiz without its answer key. The scope `quizzes:read`.
#[utoipa::path(
    get,
    path = "/plugin-api/assignments/{id}/quiz",
    tag = "plugin-api",
    security(("plugin" = [])),
    params(("id" = Uuid, Path, description = "A quiz")),
    responses(
        (status = 200, description = "The questions", body = PluginQuiz),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `quizzes:read`, or not the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such quiz", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn get_quiz(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    Path(id): Path<Uuid>,
) -> Reply<PluginQuiz> {
    let mut conn = state.db.acquire().await.map_err(db)?;
    let quiz = plugin_read::quiz(&mut conn, &caller, id).await?;
    reply(PluginQuiz {
        assignment_id: quiz.assignment_id,
        course_id: quiz.course_id,
        max_points: quiz.max_points,
        config: quiz.config,
    })
}

/// The submitted attempts at a quiz with their score per question, by the time they were
/// submitted; `updated_since` is that time. The scope `quizzes:read`.
#[utoipa::path(
    get,
    path = "/plugin-api/assignments/{id}/attempts",
    tag = "plugin-api",
    security(("plugin" = [])),
    params(("id" = Uuid, Path, description = "A quiz"), PluginListQuery),
    responses(
        (status = 200, description = "A page of attempts", body = Page<PluginAttempt>),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `quizzes:read`, or not the course",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such quiz", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 422, description = "A cursor or a time that is not valid", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn list_attempts(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    Path(id): Path<Uuid>,
    ValidQuery(query): ValidQuery<PluginListQuery>,
) -> Reply<Page<PluginAttempt>> {
    let page_params = PluginPage::new(query.cursor.as_deref(), query.limit)?;
    let mut conn = state.db.acquire().await.map_err(db)?;
    let list =
        plugin_read::attempts(&mut conn, &caller, id, query.updated_since, page_params).await?;
    reply(page(list, |a| PluginAttempt {
        id: a.id,
        student_id: a.student_id,
        attempt: a.attempt,
        scores: a.scores,
        started_at: a.started_at,
        submitted_at: a.submitted_at,
    }))
}

/// The email addresses of up to 100 people, for those of them who are in a course the plugin was
/// granted. The others are left out, not reported. The scope `users:contact`.
#[utoipa::path(
    get,
    path = "/plugin-api/users/contact",
    tag = "plugin-api",
    security(("plugin" = [])),
    params(PluginContactQuery),
    responses(
        (status = 200, description = "The addresses", body = PluginContacts),
        (status = 401, description = "No token, or one that is unknown or disabled",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "The plugin was not granted `users:contact`",
            body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`ids` is not a list of at most 100 user ids",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn get_contacts(
    caller: PluginCaller,
    State(state): State<PluginsState>,
    ValidQuery(query): ValidQuery<PluginContactQuery>,
) -> Reply<PluginContacts> {
    let ids = query
        .ids
        .split(',')
        .map(|id| id.trim().parse::<Uuid>())
        .collect::<Result<Vec<_>, _>>()
        .ok()
        .filter(|ids| ids.len() <= MAX_CONTACT_IDS)
        .ok_or_else(|| {
            ApiError::invalid_field(
                "ids",
                format!("must be at most {MAX_CONTACT_IDS} user ids separated by commas"),
            )
        })?;
    let mut conn = state.db.acquire().await.map_err(db)?;
    let contacts = plugin_read::contacts(&mut conn, &caller, &ids).await?;
    reply(PluginContacts {
        items: contacts
            .into_iter()
            .map(|c| PluginContact {
                user_id: c.user_id,
                email: c.email,
            })
            .collect(),
    })
}
