//! What a teacher tells a course (ADR-0076). Anyone who can see the course reads it; staff and
//! admins write it, as they write content. Publishing is the one change plugins are told about.

use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use super::{TeachingError, load};
use crate::teaching::model::Announcement;
use crate::teaching::repo;
use access::Actor;
use access::{self, Action};
use common::{Page, PageParams};
use events::{self, AuditEntry, Event};

/// Publishes an announcement to the course and tells the plugins (`announcement.published`) in the
/// same transaction. Staff and admins; not in an archived course.
pub async fn publish_announcement(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    title: &str,
    body: &str,
) -> Result<Announcement, TeachingError> {
    let (_, access) = load(conn, actor, course_id, true).await?;
    access::authorize(actor, Action::EditContent(access))?;
    let announcement = repo::insert_announcement(
        conn,
        Uuid::now_v7(),
        course_id,
        actor.user_id(),
        title.trim(),
        body,
    )
    .await?;
    events::emit(
        conn,
        Event::AnnouncementPublished {
            course_id,
            announcement_id: announcement.id,
        },
    )
    .await?;
    audit(conn, actor, "course.announcement.publish", &announcement).await?;
    Ok(announcement)
}

/// A course's announcements, newest first. Anyone who may view the course.
pub async fn list_announcements(
    conn: &mut PgConnection,
    actor: &Actor,
    course_id: Uuid,
    page: PageParams,
) -> Result<Page<Announcement>, TeachingError> {
    load(conn, actor, course_id, false).await?;
    let rows =
        repo::list_announcements(conn, course_id, page.after, i64::from(page.limit) + 1).await?;
    Ok(Page::from_rows(rows, page.limit, |a| a.id))
}

/// Loads an announcement together with the course it is in, as the actor sees it. An announcement
/// in a course the actor cannot see is not there.
async fn locked(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<(Announcement, access::CourseAccess), TeachingError> {
    let found = repo::find_announcement(conn, id)
        .await?
        .ok_or(TeachingError::AnnouncementNotFound)?;
    let (_, access) =
        load(conn, actor, found.course_id, true)
            .await
            .map_err(|error| match error {
                TeachingError::CourseNotFound => TeachingError::AnnouncementNotFound,
                other => other,
            })?;
    Ok((found, access))
}

/// Changes the title and/or the text. Sends nothing to the plugins. Staff and admins.
pub async fn update_announcement(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
    title: Option<&str>,
    body: Option<&str>,
) -> Result<Announcement, TeachingError> {
    let (found, access) = locked(conn, actor, id).await?;
    access::authorize(actor, Action::EditContent(access))?;
    if title.is_none() && body.is_none() {
        return Ok(found);
    }
    let updated = repo::update_announcement(conn, id, title.map(str::trim), body)
        .await?
        .ok_or(TeachingError::AnnouncementNotFound)?;
    audit(conn, actor, "course.announcement.update", &updated).await?;
    Ok(updated)
}

pub async fn delete_announcement(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<(), TeachingError> {
    let (found, access) = locked(conn, actor, id).await?;
    access::authorize(actor, Action::EditContent(access))?;
    repo::delete_announcement(conn, id).await?;
    audit(conn, actor, "course.announcement.delete", &found).await
}

async fn audit(
    conn: &mut PgConnection,
    actor: &Actor,
    action: &'static str,
    announcement: &Announcement,
) -> Result<(), TeachingError> {
    events::audit_admin(
        conn,
        actor,
        AuditEntry {
            action,
            resource_type: "course",
            resource_id: announcement.course_id,
            meta: json!({"announcement_id": announcement.id}),
        },
    )
    .await?;
    Ok(())
}
