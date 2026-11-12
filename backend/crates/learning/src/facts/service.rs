use events::{Event, SubscriberError, Subscribers};
use sqlx::PgConnection;
use uuid::Uuid;

use super::repo;
use crate::LearningError;
use crate::content::{self, FactScope};

/// Makes the facts inside `scope` what the sources say now. It holds the scope first and reads the
/// sources after, so a change it had to wait for is in what it reads. It authorizes nothing: it
/// acts on an event, or on the administrator's command.
pub async fn sync(conn: &mut PgConnection, scope: FactScope) -> Result<(), LearningError> {
    content::lock_facts(conn, &scope).await?;
    let sources = repo::sources(conn, &scope).await?;
    content::replace_source_facts(conn, &scope, &sources).await?;
    Ok(())
}

/// Makes the facts of one course, or of everything, from the sources again. The students' own marks
/// stay. It must run in a transaction (it locks the facts table, which `LOCK TABLE` allows only
/// there), and writers wait for it, so it is meant for a quiet moment.
pub async fn rebuild_facts(
    conn: &mut PgConnection,
    course_id: Option<Uuid>,
) -> Result<(), LearningError> {
    let scope = course_id.map_or(FactScope::All, FactScope::Course);
    sync(conn, scope).await
}

/// The synchronous subscribers of the facts: a submitted attempt and a change of what a student
/// can see of a grade each bring one student on one assignment up to date.
pub fn subscribers() -> Subscribers {
    Subscribers::new()
        .on("submission.submitted", |conn, event| {
            Box::pin(async move {
                if let Event::SubmissionSubmitted {
                    assignment_id,
                    student_id,
                    ..
                } = event
                {
                    sync_pair(conn, *assignment_id, *student_id).await?;
                }
                Ok(())
            })
        })
        .on("grade.publication_changed", |conn, event| {
            Box::pin(async move {
                if let Event::GradePublicationChanged {
                    assignment_id,
                    student_id,
                    ..
                } = event
                {
                    sync_pair(conn, *assignment_id, *student_id).await?;
                }
                Ok(())
            })
        })
}

async fn sync_pair(
    conn: &mut PgConnection,
    item_id: Uuid,
    student_id: Uuid,
) -> Result<(), SubscriberError> {
    sync(
        conn,
        FactScope::Pair {
            item_id,
            student_id,
        },
    )
    .await
    .map_err(SubscriberError::api)
}
