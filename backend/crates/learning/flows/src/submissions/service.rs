use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::LearningError;
use crate::files::{self, UsageOwner};
use access::Actor;
use activity::EffectiveProfiles;
use assessment::{self, Attempt, Draft, DueDraft};

/// Replaces the caller's draft with `content`. `assessment` authorizes, validates and locks the
/// draft row; the files the content names are checked and locked (`files::check`); `assessment`
/// writes the draft and attaches them in the same transaction. The lock is held until the
/// handler commits, so no one else can attach an upload that this draft took (ADR-0069).
pub async fn save_draft(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    content: &Value,
    profiles: &EffectiveProfiles,
    now: DateTime<Utc>,
) -> Result<Draft, LearningError> {
    let save =
        assessment::begin_save_draft(conn, actor, assignment_id, content, profiles, now).await?;
    let owner = UsageOwner::Submission {
        id: save.submission_id(),
        assignment_id,
    };
    let verdict = files::check(conn, actor.user_id(), &owner, save.files()).await?;
    Ok(assessment::finish_save_draft(conn, save, &verdict).await?)
}

/// Turns the caller's draft into their next attempt. The files of the draft are checked again,
/// under their locks, right before the attempt is written.
pub async fn submit(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    profiles: &EffectiveProfiles,
    now: DateTime<Utc>,
) -> Result<Attempt, LearningError> {
    let start = assessment::begin_submit(conn, actor, assignment_id, profiles, now).await?;
    let owner = UsageOwner::Submission {
        id: start.submission_id(),
        assignment_id: start.assignment_id(),
    };
    let verdict = files::check(conn, start.student_id(), &owner, start.files()).await?;
    Ok(assessment::finish_submit(conn, start, &verdict).await?)
}

/// Submits the saved draft of a student at their deadline, as the scheduler's act (ADR-0091). The
/// files are checked like a student's submit: a draft whose file is gone is refused, not submitted
/// with a dangling reference.
pub async fn auto_submit(
    conn: &mut PgConnection,
    due: &DueDraft,
    profiles: &EffectiveProfiles,
    now: DateTime<Utc>,
) -> Result<Attempt, LearningError> {
    let start = assessment::begin_auto_submit(conn, due, profiles, now).await?;
    let owner = UsageOwner::Submission {
        id: start.submission_id(),
        assignment_id: start.assignment_id(),
    };
    let verdict = files::check(conn, start.student_id(), &owner, start.files()).await?;
    Ok(assessment::finish_submit(conn, start, &verdict).await?)
}
