//! The course cards of the home page and the course list (ADR-0052).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use super::{TeachingError, access_of};
use crate::actor::Actor;
use crate::policy::{self, Action};
use crate::teaching::model::{
    CardCourse, CourseCardData, DraftItem, NextDeadline, Progress, ScopedCourse, Todo, TodoCounts,
};
use crate::teaching::repo;

/// How many titles of unpublished items a card names.
const TITLES_SHOWN: usize = 3;

/// The regular courses the actor studies in and may view (published or archived), by title. The
/// read interface that `content` uses for `GET /me/work` (ADR-0052).
pub async fn student_courses(
    conn: &mut PgConnection,
    actor: &Actor,
) -> Result<Vec<CardCourse>, TeachingError> {
    policy::authorize(actor, Action::ViewOwnCourses)?;
    let courses = repo::card_courses(conn, actor.user_id()).await?;
    Ok(courses
        .into_iter()
        .filter(|card| {
            let access = access_of(actor, &card.course, card.role);
            access.is_student() && policy::authorize(actor, Action::ViewCourse(access)).is_ok()
        })
        .collect())
}

/// The regular courses the actor may view and takes part in, as staff or as a student. The read
/// interface that `content` uses for search (ADR-0052).
pub async fn course_scope(
    conn: &mut PgConnection,
    actor: &Actor,
) -> Result<Vec<ScopedCourse>, TeachingError> {
    policy::authorize(actor, Action::ViewOwnCourses)?;
    let courses = repo::card_courses(conn, actor.user_id()).await?;
    Ok(courses
        .into_iter()
        .filter_map(|card| {
            let access = access_of(actor, &card.course, card.role);
            policy::authorize(actor, Action::ViewCourse(access)).ok()?;
            let staff = policy::authorize(actor, Action::ManageContent(access)).is_ok();
            Some(ScopedCourse {
                course: card,
                staff,
            })
        })
        .collect())
}

/// One card per regular course the actor takes part in, by title. A student's card carries their
/// progress and next deadline, a staff card its to-do list. The number of courses of one person is
/// small, so the list is not paged.
pub async fn course_cards(
    conn: &mut PgConnection,
    actor: &Actor,
    now: DateTime<Utc>,
) -> Result<Vec<CourseCardData>, TeachingError> {
    policy::authorize(actor, Action::ViewOwnCourses)?;
    let courses = repo::card_courses(conn, actor.user_id()).await?;
    let mut staff = Vec::new();
    let mut student = Vec::new();
    let mut shown = Vec::new();
    for card in courses {
        let access = access_of(actor, &card.course, card.role);
        if policy::authorize(actor, Action::ViewCourse(access)).is_err() {
            continue;
        }
        if policy::authorize(actor, Action::ManageContent(access)).is_ok() {
            staff.push(card.course.id);
        } else if access.is_student() {
            student.push(card.course.id);
        }
        shown.push((card, access.is_owner));
    }

    let progress: HashMap<Uuid, Progress> =
        repo::student_progress(conn, actor.user_id(), &student, now)
            .await?
            .into_iter()
            .map(|p| (p.course_id, p))
            .collect();
    let mut next: HashMap<Uuid, NextDeadline> =
        repo::student_next(conn, actor.user_id(), &student, now)
            .await?
            .into_iter()
            .map(|n| (n.course_id, n))
            .collect();
    let counts: HashMap<Uuid, TodoCounts> = repo::staff_todo_counts(conn, &staff)
        .await?
        .into_iter()
        .map(|c| (c.course_id, c))
        .collect();
    let mut drafts: HashMap<Uuid, Vec<DraftItem>> = HashMap::new();
    for item in repo::unpublished_items(conn, &staff).await? {
        drafts.entry(item.course_id).or_default().push(item);
    }
    let mut deadlines: HashMap<Uuid, NextDeadline> = repo::staff_next_deadline(conn, &staff, now)
        .await?
        .into_iter()
        .map(|n| (n.course_id, n))
        .collect();

    Ok(shown
        .into_iter()
        .map(|(card, owner)| {
            let id = card.course.id;
            if student.contains(&id) {
                CourseCardData {
                    course: card,
                    owner,
                    progress: Some(progress.get(&id).cloned().unwrap_or(Progress {
                        course_id: id,
                        submitted: 0,
                        total: 0,
                        graded: 0,
                    })),
                    next: next.remove(&id),
                    todo: Vec::new(),
                }
            } else {
                let mut todo = Vec::new();
                if let Some(c) = counts.get(&id) {
                    if c.to_grade > 0 {
                        todo.push(Todo::ToGrade { count: c.to_grade });
                    }
                    if c.draft_grades > 0 {
                        todo.push(Todo::DraftGrades {
                            count: c.draft_grades,
                            auto_scored: c.from_quizzes,
                        });
                    }
                }
                if let Some(items) = drafts.get(&id) {
                    todo.push(Todo::UnpublishedItems {
                        count: i64::try_from(items.len()).unwrap_or(i64::MAX),
                        titles: items
                            .iter()
                            .take(TITLES_SHOWN)
                            .map(|item| item.title.clone())
                            .collect(),
                    });
                }
                if let Some(deadline) = deadlines.remove(&id) {
                    todo.push(Todo::NextDeadline(deadline));
                }
                CourseCardData {
                    course: card,
                    owner,
                    progress: None,
                    next: None,
                    todo,
                }
            }
        })
        .collect())
}
