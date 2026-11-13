//! The roster of a course changes when people come to or go from a group it uses. `org` knows the
//! group and not the courses, so it writes `group.members_changed` and this handler, a flow of
//! the layer above, tells the plugins which rosters it touches (ADR-0075).

use events::{Event, HandlerError, Handlers, OutboxEvent};
use sqlx::PgPool;
use uuid::Uuid;

const GROUP_MEMBERS_CHANGED: &str = "group.members_changed";

/// The outbox handler of `group.members_changed`: one `enrollment.changed` for every user and
/// every course that uses the group (or one of its subgroups), in one transaction. A repeat of the
/// handler repeats the events, which a plugin takes as a reason to read the roster again.
pub fn event_handlers(pool: PgPool) -> Handlers {
    Handlers::default().on(GROUP_MEMBERS_CHANGED, move |event| {
        let pool = pool.clone();
        Box::pin(async move { announce(&pool, &event).await.map_err(HandlerError) })
    })
}

fn ids(event: &OutboxEvent) -> Option<(Uuid, Vec<Uuid>)> {
    let group = event.payload["group_id"].as_str()?.parse().ok()?;
    let users = event.payload["user_ids"]
        .as_array()?
        .iter()
        .filter_map(|id| id.as_str()?.parse().ok())
        .collect();
    Some((group, users))
}

async fn announce(pool: &PgPool, event: &OutboxEvent) -> Result<(), String> {
    let (group_id, users) = ids(event).ok_or("the event names no group")?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let courses = teaching::linked_courses_of_group(&mut tx, group_id)
        .await
        .map_err(|e| e.to_string())?;
    for course in courses {
        for &user_id in &users {
            events::emit(
                &mut tx,
                Event::EnrollmentChanged {
                    course_id: course.course_id,
                    user_id,
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().await.map_err(|e| e.to_string())
}
