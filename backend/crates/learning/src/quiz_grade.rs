//! The draft grade of a quiz (ADR-0050 point 5, ADR-0093). A submitted attempt that was scored in
//! full leaves its sum on the attempt, and this subscriber of `quiz.attempt_submitted` turns the
//! sum into the draft grade, in the transaction of the submit. `assessment` sits below `gradebook`
//! and does not call it: the flow does, above both.

use events::{Event, SubscriberError, Subscribers};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{assessment, gradebook};

/// Writes the draft grade of the attempt, when it has a total. An attempt with a question left for
/// a person has none, and the teacher grades the whole by hand.
async fn grade_attempt(
    conn: &mut PgConnection,
    submission_id: Uuid,
) -> Result<(), SubscriberError> {
    let failed = |error: &dyn std::fmt::Display| SubscriberError(error.to_string());
    let Some(attempt) = assessment::auto_total(conn, submission_id)
        .await
        .map_err(|e| failed(&e))?
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
    .map_err(|e| failed(&e))
}

pub fn subscribers() -> Subscribers {
    Subscribers::new().on("quiz.attempt_submitted", |conn, event| {
        Box::pin(async move {
            if let Event::QuizAttemptSubmitted { submission_id, .. } = event {
                grade_attempt(conn, *submission_id).await?;
            }
            Ok(())
        })
    })
}
