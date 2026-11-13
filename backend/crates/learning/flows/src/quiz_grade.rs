//! The draft grade of a quiz (ADR-0050 point 5, ADR-0093). A submitted attempt that was scored in
//! full leaves its sum on the attempt, and this subscriber of `submission.submitted` turns the sum
//! into the draft grade, in the transaction of the submit. `assessment` sits below `gradebook` and
//! does not call it: the flow does, above both.
//!
//! It listens to `submission.submitted` and not to `quiz.attempt_submitted` on purpose, for the
//! order of the locks. Publishing a grade takes the grade rows first and the facts of the pair
//! next, so a submit has to do the same: this subscriber (the grade row) is registered before the
//! facts one (the pair), see [`crate::subscribers`]. An attempt with no total is skipped.

use events::{Event, SubscriberError, Subscribers};
use sqlx::PgConnection;
use uuid::Uuid;

/// Writes the draft grade of the attempt, when it has a total. An attempt with a question left for
/// a person (or a text or form attempt) has none, and the teacher grades the whole by hand.
async fn grade_attempt(
    conn: &mut PgConnection,
    submission_id: Uuid,
) -> Result<(), SubscriberError> {
    let Some(attempt) = assessment::auto_total(conn, submission_id)
        .await
        .map_err(SubscriberError::api)?
    else {
        return Ok(());
    };
    gradebook::save_auto_grade(
        conn,
        attempt.assignment_id,
        attempt.student_id,
        attempt.total,
    )
    .await
    .map_err(SubscriberError::api)
}

pub fn subscribers() -> Subscribers {
    Subscribers::new().on("submission.submitted", |conn, event| {
        Box::pin(async move {
            if let Event::SubmissionSubmitted { submission_id, .. } = event {
                grade_attempt(conn, *submission_id).await?;
            }
            Ok(())
        })
    })
}
