//! HTTP edge of the work read models: the facts of a course's items and a student's work list
//! (ADR-0049, ADR-0052).

use api_types::{ItemMeta, ItemMetaMap, MyWork, ProblemDetails, WorkCourse, WorkItem};
use axum::Json;
use axum::extract::{Path, State};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::GradebookState;
use crate::model;
use crate::service::{self, GradebookError};
use access::Actor;
use common::ApiError;

pub fn router<S>() -> OpenApiRouter<S>
where
    S: access::ActorSource + Clone + 'static,
    GradebookState: axum::extract::FromRef<S>,
{
    OpenApiRouter::new()
        .routes(routes!(get_item_meta))
        .routes(routes!(get_my_work))
}

/// Counts are far below `u32::MAX`; a database that disagreed would still not overflow the field.
fn count(n: i64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn api_work_status(status: model::WorkStatus) -> api_types::WorkStatus {
    match status {
        model::WorkStatus::NotStarted => api_types::WorkStatus::NotStarted,
        model::WorkStatus::Draft => api_types::WorkStatus::Draft,
        model::WorkStatus::Submitted => api_types::WorkStatus::Submitted,
        model::WorkStatus::Graded => api_types::WorkStatus::Graded,
        model::WorkStatus::Missed => api_types::WorkStatus::Missed,
        model::WorkStatus::InClass => api_types::WorkStatus::InClass,
    }
}

fn api_meta(facts: model::ItemMeta) -> ItemMeta {
    let mut meta = ItemMeta {
        read_minutes: facts.content.read_minutes.map(count),
        class_on: facts.content.class_on,
        edited: facts.content.edited,
        ..ItemMeta::default()
    };
    if let Some(work) = facts.work {
        meta.status = Some(api_work_status(work.status));
        meta.score = work.score.map(|score| format!("{score:.2}"));
        meta.draft_saved_at = work.draft_saved_at;
        meta.published_at = work.published_at;
        meta.attempts_used = Some(count(work.attempts_used));
    }
    if let Some(staff) = facts.staff {
        meta.submitted = Some(count(staff.submitted));
        meta.to_grade = Some(count(staff.to_grade));
        meta.draft_grades = Some(count(staff.draft_grades));
        meta.graded_total = Some(count(staff.graded_total));
    }
    meta
}

/// The facts of every item of the course that the tree does not carry, by item id.
///
/// Everyone gets the reading time of a lecture and the class day of in-class work. Staff also get
/// `edited` and the counts of what to grade; a student gets their own standing, with published
/// values only.
#[utoipa::path(
    get,
    path = "/courses/{id}/item-meta",
    tag = "content",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Course id")),
    responses(
        (status = 200, description = "The facts by item id", body = ItemMetaMap),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
        (status = 404, description = "No such course, or not visible to the caller",
            body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
async fn get_item_meta(
    State(state): State<GradebookState>,
    actor: Actor,
    Path(id): Path<Uuid>,
) -> Result<Json<ItemMetaMap>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(GradebookError::from)?;
    let facts = service::item_meta(&mut conn, &actor, id).await?;
    Ok(Json(ItemMetaMap {
        items: facts
            .into_iter()
            .map(|(id, facts)| (id.to_string(), api_meta(facts)))
            .collect(),
    }))
}

/// Every assignment of the courses the caller studies in, with the caller's own standing on it
/// (ADR-0052). Only what students can open, with published titles, scores and times.
#[utoipa::path(
    get,
    path = "/me/work",
    tag = "content",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The caller's courses and work", body = MyWork),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
async fn get_my_work(
    State(state): State<GradebookState>,
    actor: Actor,
) -> Result<Json<MyWork>, ApiError> {
    let mut conn = state.db.acquire().await.map_err(GradebookError::from)?;
    let work = service::my_work(&mut conn, &actor, chrono::Utc::now()).await?;
    Ok(Json(MyWork {
        courses: work
            .courses
            .into_iter()
            .map(|card| WorkCourse {
                id: card.course.id,
                code: card.code,
                title: card.course.title,
                term_name: card.term_name,
                owner_name: card.owner_name,
                owner_surname: card.owner_surname,
            })
            .collect(),
        items: work
            .items
            .into_iter()
            .map(|entry| WorkItem {
                assignment_id: entry.assignment_id,
                course_id: entry.course_id,
                title: entry.title,
                assignment_type: entry.assignment_type.id().to_owned(),
                due_at: entry.due_at,
                class_on: entry.class_on.map(|day| day.to_string()),
                max_points: entry.max_points.to_string(),
                status: api_work_status(entry.status),
                score: entry.score.map(|score| format!("{score:.2}")),
                published_at: entry.published_at,
                draft_saved_at: entry.draft_saved_at,
            })
            .collect(),
    }))
}
