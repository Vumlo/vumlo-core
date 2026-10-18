//! `GET /me/course-cards` (ADR-0052).

use api_types::{
    CardNext, CardProgress, CardRole, CardTodo, CourseCard, CourseCards, ProblemDetails,
};
use axum::Json;
use axum::extract::State;
use chrono::Utc;

use super::api_course;
use crate::actor::Actor;
use crate::app::AppState;
use crate::error::ApiError;
use crate::policy::CourseRole;
use crate::teaching::model::{CourseCardData, NextDeadline, Todo};
use crate::teaching::service::{self, TeachingError};

fn api_todo(todo: Todo) -> CardTodo {
    match todo {
        Todo::ToGrade { count } => CardTodo::ToGrade { count },
        Todo::DraftGrades { count, auto_scored } => CardTodo::DraftGrades { count, auto_scored },
        Todo::UnpublishedItems { count, titles } => CardTodo::UnpublishedItems { count, titles },
        Todo::NextDeadline(NextDeadline {
            assignment_id,
            title,
            due_at,
            ..
        }) => CardTodo::NextDeadline {
            assignment_id,
            title,
            due_at,
        },
    }
}

fn api_card(card: CourseCardData, today: chrono::NaiveDate) -> CourseCard {
    let role = if card.owner {
        CardRole::Owner
    } else {
        match card.course.role {
            Some(CourseRole::Teacher) => CardRole::Teacher,
            Some(CourseRole::Assistant) => CardRole::Assistant,
            _ => CardRole::Student,
        }
    };
    let course = card.course;
    CourseCard {
        term_ended: course.term_ends.is_some_and(|ends| ends < today),
        course: api_course(course.course),
        code: course.code,
        term_name: course.term_name,
        owner_name: course.owner_name,
        owner_surname: course.owner_surname,
        my_role: role,
        student_count: course.student_count,
        assistant_count: course.assistant_count,
        progress: card.progress.map(|p| CardProgress {
            submitted: p.submitted,
            total: p.total,
            graded: p.graded,
        }),
        next: card.next.map(|n| CardNext {
            assignment_id: n.assignment_id,
            title: n.title,
            due_at: n.due_at,
        }),
        todo: card.todo.into_iter().map(api_todo).collect(),
    }
}

/// One card for each regular course the caller takes part in, by title: a student's card has their
/// progress and next deadline, a teacher's has a to-do list with counts and kinds, not sentences.
/// A student sees only published and archived courses, and only what students may open in them.
#[utoipa::path(
    get,
    path = "/me/course-cards",
    tag = "courses",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The caller's cards", body = CourseCards),
        (status = 401, description = "Not logged in", body = ProblemDetails,
            content_type = "application/problem+json"),
    )
)]
pub async fn get_course_cards(
    State(state): State<AppState>,
    actor: Actor,
) -> Result<Json<CourseCards>, ApiError> {
    let now = Utc::now();
    let today = now
        .with_timezone(&state.config.institution.time_zone)
        .date_naive();
    let mut conn = state.db.acquire().await.map_err(TeachingError::from)?;
    let cards = service::course_cards(&mut conn, &actor, now).await?;
    Ok(Json(CourseCards {
        items: cards.into_iter().map(|c| api_card(c, today)).collect(),
    }))
}
