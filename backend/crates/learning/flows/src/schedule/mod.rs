//! The scheduler (ADR-0091): the work that is due at a time and not at a request. Two jobs, each
//! safe to run twice and from two workers, because each leaves a mark on the row it works on:
//!
//! 1. an assignment that became visible to students gets its `assignment.created` event
//!    (`content::announce_visible`), which a publish route already does for the moment it
//!    publishes; this catches a `publish_at` that has passed, or one a PATCH cleared;
//! 2. a draft whose assignment says `on_deadline = auto_submit` is submitted once `due_at` has
//!    passed (`submissions::auto_submit`).
//!
//! It is a flow here because it needs `content` and `assessment` both. Like the orphan sweep it
//! takes the time as a parameter, so a test runs a tick at the moment it chooses.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::assessment::{self, AssessmentError, DueDraft};
use crate::content::{self, ContentError, EffectiveProfiles};
use crate::{LearningError, submissions};

/// How often the scheduler wakes. A `publish_at` or a deadline is acted on at most this late.
const INTERVAL: Duration = Duration::from_secs(30);
/// The drafts one tick submits at most. The rest wait for the next tick.
// ponytail: a deadline with more drafts than this takes several ticks; raise it or run the
// submits in parallel if a course ever needs the whole class submitted within one interval.
const BATCH: i64 = 200;

#[derive(Debug, thiserror::Error)]
pub enum ScheduleError {
    #[error(transparent)]
    Content(#[from] ContentError),
    #[error(transparent)]
    Assessment(#[from] AssessmentError),
    #[error(transparent)]
    Learning(#[from] LearningError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// What one tick did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tick {
    /// Assignments announced to the plugins.
    pub announced: usize,
    /// Drafts submitted at their deadline.
    pub submitted: usize,
    /// Drafts the submit refused; they are kept and marked.
    pub skipped: usize,
}

enum Outcome {
    Submitted,
    Skipped,
    /// The draft or the setting changed since it was listed: nothing to do.
    Gone,
}

/// Runs both jobs as of `now`. The announce is one transaction. Each draft is submitted in a
/// transaction of its own, so one that fails does not hold back the rest of the class; a failure
/// of a single draft is logged by id and the tick goes on.
pub async fn tick(
    db: &PgPool,
    profiles: &EffectiveProfiles,
    now: DateTime<Utc>,
) -> Result<Tick, ScheduleError> {
    let mut report = Tick::default();
    let mut tx = db.begin().await?;
    report.announced = content::announce_visible(&mut tx, now, None).await?;
    tx.commit().await?;

    let due = {
        let mut conn = db.acquire().await?;
        assessment::due_drafts(&mut conn, now, BATCH).await?
    };
    for draft in due {
        match submit_one(db, profiles, &draft, now).await {
            Ok(Outcome::Submitted) => report.submitted += 1,
            Ok(Outcome::Skipped) => report.skipped += 1,
            Ok(Outcome::Gone) => {}
            Err(error) => tracing::error!(
                assignment_id = %draft.assignment_id,
                student_id = %draft.student_id,
                %error,
                "auto-submitting a draft failed; it is tried again on the next tick"
            ),
        }
    }
    Ok(report)
}

async fn submit_one(
    db: &PgPool,
    profiles: &EffectiveProfiles,
    draft: &DueDraft,
    now: DateTime<Utc>,
) -> Result<Outcome, ScheduleError> {
    let mut tx = db.begin().await?;
    match submissions::auto_submit(&mut tx, draft, profiles, now).await {
        Ok(_) => {
            tx.commit().await?;
            Ok(Outcome::Submitted)
        }
        Err(LearningError::Assessment(
            AssessmentError::NoDraft | AssessmentError::NotDue | AssessmentError::Concurrent,
        )) => Ok(Outcome::Gone),
        Err(LearningError::Assessment(error)) if error.refuses_submit() => {
            // The refused submit may have written part of its work; none of it is kept.
            tx.rollback().await?;
            let mut tx = db.begin().await?;
            assessment::skip_auto_submit(&mut tx, draft, now).await?;
            tx.commit().await?;
            Ok(Outcome::Skipped)
        }
        // Dropping the transaction rolls it back.
        Err(error) => Err(error.into()),
    }
}

/// Runs a tick every 30 seconds until the process ends. Never returns and never panics: a failed
/// round is logged and the next one runs on time.
pub async fn run_scheduler(db: PgPool, profiles: Arc<EffectiveProfiles>) {
    loop {
        match tick(&db, &profiles, Utc::now()).await {
            Ok(done) if done == Tick::default() => {}
            Ok(done) => tracing::info!(
                announced = done.announced,
                submitted = done.submitted,
                skipped = done.skipped,
                "the scheduler did work"
            ),
            Err(error) => tracing::error!(%error, "the scheduler failed a round"),
        }
        tokio::time::sleep(INTERVAL).await;
    }
}
