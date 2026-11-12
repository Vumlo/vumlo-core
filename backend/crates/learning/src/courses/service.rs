use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{CourseCardData, Todo};
use crate::LearningError;
use crate::content::{self, DraftItem, NextDeadline};
use crate::gradebook::{self, Progress, TodoCounts};
use crate::teaching;
use access::Actor;

/// How many titles of unpublished items a card names.
const TITLES_SHOWN: usize = 3;

/// One card per regular course the actor takes part in, by title. A student's card carries their
/// progress and next deadline, a staff card its to-do list. `teaching` says which courses the
/// actor sees and on which side; `gradebook` and `content` are asked only about those, so a
/// student's figures are published values and the drafts reach staff cards alone. The number of
/// courses of one person is small, so the list is not paged.
pub async fn cards(
    conn: &mut PgConnection,
    actor: &Actor,
    now: DateTime<Utc>,
) -> Result<Vec<CourseCardData>, LearningError> {
    let shown = teaching::course_scope(conn, actor).await?;
    let staff: Vec<Uuid> = shown
        .iter()
        .filter(|scoped| scoped.staff)
        .map(|scoped| scoped.course.course.id)
        .collect();
    let student: Vec<Uuid> = shown
        .iter()
        .filter(|scoped| !scoped.staff && scoped.student)
        .map(|scoped| scoped.course.course.id)
        .collect();

    let progress: HashMap<Uuid, Progress> =
        gradebook::student_progress(conn, actor.user_id(), &student, now)
            .await?
            .into_iter()
            .map(|p| (p.course_id, p))
            .collect();
    let mut next: HashMap<Uuid, NextDeadline> =
        gradebook::student_next(conn, actor.user_id(), &student, now)
            .await?
            .into_iter()
            .map(|n| (n.course_id, n))
            .collect();
    let counts: HashMap<Uuid, TodoCounts> = gradebook::todo_counts(conn, &staff)
        .await?
        .into_iter()
        .map(|c| (c.course_id, c))
        .collect();
    let mut drafts: HashMap<Uuid, Vec<DraftItem>> = HashMap::new();
    for item in content::draft_items(conn, &staff).await? {
        drafts.entry(item.course_id).or_default().push(item);
    }
    let mut deadlines: HashMap<Uuid, NextDeadline> = content::next_deadlines(conn, &staff, now)
        .await?
        .into_iter()
        .map(|n| (n.course_id, n))
        .collect();

    Ok(shown
        .into_iter()
        .map(|scoped| {
            let id = scoped.course.course.id;
            if student.contains(&id) {
                CourseCardData {
                    course: scoped.course,
                    owner: scoped.owner,
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
                    course: scoped.course,
                    owner: scoped.owner,
                    progress: None,
                    next: None,
                    todo,
                }
            }
        })
        .collect())
}
