//! Business rules. Every function that acts for a user authorizes first, takes the connection it
//! runs on and never begins or commits a transaction.
//!
//! An assignment the actor may not see does not exist as far as they can tell (404): an unknown
//! id, an id in a course they cannot see and a draft they may not open give the same error. Only
//! after that does the action decide between going ahead and 403.
//!
//! The time is a parameter of every function that depends on it: the handler reads the clock once.
//! `assessment` calls no other module's Rust code except the read-only functions that every module
//! uses to open and lock an assignment (ADR-0030, ADR-0035).

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{
    Attempt, Draft, FormFacts, MySubmission, QuizResult, Responses, StudentSubmission, Window,
};
use super::repo;
use crate::actor::Actor;
use crate::content::{
    self, Assignment, AssignmentType, AutoGrade, BuiltinType, DocumentErrors, EffectiveProfiles,
    OpenError, OpenedAssignment, Stage,
};
use crate::events::{self, Event, EventsError};
use crate::gradebook::{self, GradebookError};
use crate::pagination::{Page, PageParams};
use crate::policy::{self, Action, PolicyError};
use crate::teaching::TeachingError;

#[derive(Debug, thiserror::Error)]
pub enum AssessmentError {
    #[error(transparent)]
    Forbidden(#[from] PolicyError),
    /// No such assignment, or the actor may not see it.
    #[error("assignment not found")]
    AssignmentNotFound,
    /// The user is neither a current student of the course nor the author of an attempt.
    #[error("student not found")]
    StudentNotFound,
    /// The assignment's type takes no submissions (`no_submission`).
    #[error("this assignment takes no submissions")]
    TakesNoSubmissions,
    /// The window closed at `due_at` or `late_until` (ADR-0035 point 5).
    #[error("the submission window is closed")]
    WindowClosed,
    /// A submit without a draft: a second submit, or nothing was saved.
    #[error("there is no draft to submit")]
    NoDraft,
    /// The student has used all their attempts.
    #[error("the attempt limit is reached")]
    AttemptLimit,
    /// Another submit of the same student got the attempt number first.
    #[error("a concurrent submit took this attempt")]
    Concurrent,
    /// The content is not a valid document of the student profile as the assignment narrows it.
    #[error("the content is not a valid document")]
    Document(DocumentErrors),
    /// A submit of a document with no text.
    #[error("the content is empty")]
    Empty,
    /// A purge found submitted attempts: the caller had to refuse the delete instead.
    #[error("there are submitted attempts")]
    HasSubmissions,
    /// An exclusive option of a form was chosen by another student first (ADR-0051 point 4).
    #[error("an exclusive option was taken by someone else")]
    OptionTaken,
    /// The result of a quiz or the responses of a form asked for another type, or the result
    /// before the grade is published (ADR-0050 point 8).
    #[error("nothing to show")]
    NothingToShow,
    #[error(transparent)]
    Gradebook(#[from] GradebookError),
    #[error(transparent)]
    Teaching(TeachingError),
    #[error(transparent)]
    Events(#[from] EventsError),
    #[error(transparent)]
    Db(sqlx::Error),
}

impl From<sqlx::Error> for AssessmentError {
    /// The unique index on the attempt number is the backstop of the submit lock.
    fn from(error: sqlx::Error) -> Self {
        match error.as_database_error().and_then(|e| e.code()).as_deref() {
            Some("23505") => Self::Concurrent,
            _ => Self::Db(error),
        }
    }
}

/// Loads an assignment the actor may view, with their access to its course.
async fn open(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
) -> Result<OpenedAssignment, AssessmentError> {
    content::open_assignment(conn, actor, assignment_id)
        .await
        .map_err(|error| match error {
            OpenError::NotFound => AssessmentError::AssignmentNotFound,
            OpenError::Teaching(error) => AssessmentError::Teaching(error),
            OpenError::Db(error) => AssessmentError::Db(error),
        })
}

/// A type that takes no submissions has nothing to save, read or list. The type cannot change, so
/// the answer from the unlocked read is safe.
fn require_submissions(opened: &OpenedAssignment) -> Result<(), AssessmentError> {
    if opened.assignment_type.takes_submissions() {
        Ok(())
    } else {
        Err(AssessmentError::TakesNoSubmissions)
    }
}

/// Takes the assignment row `FOR SHARE` and returns it as it is in the row, with its settings and
/// `config`. A change of the settings and a delete take the row `FOR UPDATE`, so they wait for
/// this transaction, and what they committed before is what this reads (ADR-0035 point 12).
async fn lock(conn: &mut PgConnection, assignment_id: Uuid) -> Result<Assignment, AssessmentError> {
    content::lock_assignment_shared(conn, assignment_id)
        .await?
        .ok_or(AssessmentError::AssignmentNotFound)
}

/// Whether a save or a submit is taken at `now`, and whether it is late.
fn open_window(assignment: &Assignment, now: DateTime<Utc>) -> Result<bool, AssessmentError> {
    match Window::at(&assignment.settings, now) {
        Window::Open { late } => Ok(late),
        Window::Closed => Err(AssessmentError::WindowClosed),
    }
}

/// Replaces the caller's draft with `content`; the last write wins and nothing is versioned.
/// Writes no event (ADR-0007).
///
/// The assignment row is locked `FOR SHARE` before the settings are read from it, so a change of
/// the window made by a transaction that began first is seen, and one that comes later waits.
pub async fn save_draft(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    content: &Value,
    profiles: &EffectiveProfiles,
    now: DateTime<Utc>,
) -> Result<Draft, AssessmentError> {
    let opened = open(conn, actor, assignment_id).await?;
    policy::authorize(actor, Action::SubmitWork(opened.access))?;
    require_submissions(&opened)?;
    let assignment = lock(conn, assignment_id).await?;
    open_window(&assignment, now)?;
    let valid = assignment
        .assignment_type
        .validate_submission(&assignment.config, content, Stage::Draft, profiles)
        .map_err(AssessmentError::Document)?;
    Ok(repo::upsert_draft(conn, assignment_id, actor.user_id(), valid.as_body()).await?)
}

/// The caller's own work: the draft and a page of attempts, newest first.
pub async fn my_submission(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    page: PageParams,
) -> Result<MySubmission, AssessmentError> {
    let opened = open(conn, actor, assignment_id).await?;
    policy::authorize(actor, Action::ViewOwnSubmission(opened.access))?;
    require_submissions(&opened)?;
    let draft = repo::find_draft(conn, assignment_id, actor.user_id()).await?;
    let rows = repo::list_attempts(
        conn,
        assignment_id,
        actor.user_id(),
        page.after,
        i64::from(page.limit) + 1,
    )
    .await?;
    Ok(MySubmission {
        draft,
        attempts: Page::from_rows(rows, page.limit, |attempt| attempt.id),
    })
}

/// Deletes the caller's draft; with none it does nothing. Allowed after the window closed: it is
/// not a save (ADR-0035). Writes no event.
pub async fn discard_draft(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
) -> Result<(), AssessmentError> {
    let opened = open(conn, actor, assignment_id).await?;
    policy::authorize(actor, Action::SubmitWork(opened.access))?;
    require_submissions(&opened)?;
    lock(conn, assignment_id).await?;
    repo::delete_draft(conn, assignment_id, actor.user_id()).await?;
    Ok(())
}

/// Turns the caller's draft into their next attempt and emits `submission.submitted`.
///
/// Order: the assignment row is locked `FOR SHARE` and the window is judged on the settings in it;
/// then the draft row is locked `FOR UPDATE`, so a second submit of the same draft waits and then
/// finds no draft; then, in a later statement, the attempts are counted. The draft row becomes the
/// attempt in place.
pub async fn submit(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    profiles: &EffectiveProfiles,
    now: DateTime<Utc>,
) -> Result<Attempt, AssessmentError> {
    let opened = open(conn, actor, assignment_id).await?;
    policy::authorize(actor, Action::SubmitWork(opened.access))?;
    require_submissions(&opened)?;
    let assignment = lock(conn, assignment_id).await?;
    let is_late = open_window(&assignment, now)?;
    let student_id = actor.user_id();
    let draft = repo::lock_draft(conn, assignment_id, student_id)
        .await?
        .ok_or(AssessmentError::NoDraft)?;
    let last = repo::last_attempt_number(conn, assignment_id, student_id).await?;
    if assignment
        .settings
        .max_attempts
        .is_some_and(|max| last >= max)
    {
        return Err(AssessmentError::AttemptLimit);
    }
    // The draft is judged again against the profile and the features as they are now.
    let valid = assignment
        .assignment_type
        .validate_submission(
            &assignment.config,
            &Value::Object(draft.content),
            Stage::Submit,
            profiles,
        )
        .map_err(AssessmentError::Document)?;
    if valid.is_blank() {
        return Err(AssessmentError::Empty);
    }
    check_claims(
        conn,
        &assignment,
        assignment_id,
        opened.course_id,
        student_id,
        valid.as_body(),
    )
    .await?;
    let auto = assignment
        .assignment_type
        .auto_grade(&assignment.config, valid.as_body());
    let scores = auto.as_ref().map(AutoGrade::scores_json);
    let attempt =
        repo::submit_draft(conn, draft.id, last + 1, now, is_late, scores.as_ref()).await?;
    if let Some(auto) = auto.filter(|auto| auto.complete) {
        gradebook::save_auto_grade(conn, assignment_id, student_id, auto.total).await?;
    }
    events::emit(
        conn,
        Event::SubmissionSubmitted {
            assignment_id,
            student_id,
            course_id: opened.course_id,
            submission_id: attempt.id,
        },
    )
    .await?;
    Ok(attempt)
}

/// What a submission asks of the rest of the course: the people it names must be current students
/// other than the submitter, and an exclusive option must not be held by another student. The
/// submits of one form are serialized by an advisory lock around the check (ADR-0051 point 4).
async fn check_claims(
    conn: &mut PgConnection,
    assignment: &Assignment,
    assignment_id: Uuid,
    course_id: Uuid,
    student_id: Uuid,
    body: &Map<String, Value>,
) -> Result<(), AssessmentError> {
    let claims = assignment.assignment_type.claims(&assignment.config, body);
    if claims.is_empty() {
        return Ok(());
    }
    if !claims.people.is_empty() {
        let mut ids = claims.people.clone();
        ids.sort();
        ids.dedup();
        if ids.contains(&student_id) {
            return Err(AssessmentError::Document(DocumentErrors::one(
                "answers",
                "must not name the student who submits",
            )));
        }
        if repo::students_among(conn, course_id, &ids).await?.len() != ids.len() {
            return Err(AssessmentError::Document(DocumentErrors::one(
                "answers",
                "names someone who is not a student of the course",
            )));
        }
    }
    if !claims.exclusive.is_empty() {
        repo::lock_form(conn, assignment_id).await?;
        for other in repo::latest_contents(conn, assignment_id).await? {
            if other.student.user_id == student_id {
                continue;
            }
            let held = assignment
                .assignment_type
                .claims(&assignment.config, &other.content)
                .exclusive;
            if claims.exclusive.iter().any(|claim| held.contains(claim)) {
                return Err(AssessmentError::OptionTaken);
            }
        }
    }
    Ok(())
}

/// The facts a form's config shows besides what was configured: which fields have answers and who
/// holds each exclusive option (ADR-0051 point 5). The caller has authorized the read.
pub async fn form_facts(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    assignment_type: BuiltinType,
    config: &Map<String, Value>,
) -> Result<FormFacts, AssessmentError> {
    let mut facts = FormFacts::default();
    for latest in repo::latest_contents(conn, assignment_id).await? {
        if let Some(answers) = latest.content.get("answers").and_then(Value::as_object) {
            facts.answered.extend(answers.keys().cloned());
        }
        let name = format!("{} {}", latest.student.name, latest.student.surname);
        for (field, option) in assignment_type.claims(config, &latest.content).exclusive {
            facts
                .holders
                .insert((field, option), (latest.student.user_id, name.clone()));
        }
    }
    Ok(facts)
}

/// The caller's result of a quiz: their latest attempt, the server's score of each question and the
/// published grade. Only once a score is published; before that, and for any other type, there is
/// nothing to show (ADR-0050 point 8).
pub async fn quiz_result(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
) -> Result<QuizResult, AssessmentError> {
    let opened = open(conn, actor, assignment_id).await?;
    policy::authorize(actor, Action::ViewOwnGrades(opened.access))?;
    if opened.assignment_type != BuiltinType::Quiz {
        return Err(AssessmentError::NothingToShow);
    }
    let assignment = lock(conn, assignment_id).await?;
    let published = gradebook::published_grade(conn, assignment_id, actor.user_id())
        .await?
        .ok_or(AssessmentError::NothingToShow)?;
    let score = published.score.ok_or(AssessmentError::NothingToShow)?;
    let (attempt, scores) = repo::latest_with_scores(conn, assignment_id, actor.user_id())
        .await?
        .ok_or(AssessmentError::NothingToShow)?;
    let questions = content::quiz_result_questions(&assignment.config, &attempt.content, &scores)
        .ok_or(AssessmentError::NothingToShow)?;
    Ok(QuizResult {
        score,
        max_points: assignment.max_points.value(),
        published_at: published.published_at,
        feedback: published.feedback,
        attempt: attempt.attempt,
        questions,
    })
}

/// The responses to a form: every current student's latest attempt, and who has none. Staff and
/// admins (ADR-0051 point 7).
pub async fn responses(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
) -> Result<Responses, AssessmentError> {
    let opened = open(conn, actor, assignment_id).await?;
    policy::authorize(actor, Action::ViewSubmissions(opened.access))?;
    if opened.assignment_type != BuiltinType::Form {
        return Err(AssessmentError::NothingToShow);
    }
    let students = repo::current_students(conn, opened.course_id).await?;
    let mut latest = repo::latest_contents(conn, assignment_id).await?;
    let mut rows = Vec::new();
    let mut missing = Vec::new();
    for student in &students {
        match latest
            .iter()
            .position(|row| row.student.user_id == student.user_id)
        {
            Some(index) => rows.push(latest.swap_remove(index)),
            None => missing.push(student.clone()),
        }
    }
    Ok(Responses {
        total_students: i64::try_from(students.len()).unwrap_or(i64::MAX),
        missing,
        rows,
    })
}

/// The current students of the course with their latest submitted attempt, in roster order.
/// Staff and admins. No draft is read.
pub async fn list_submissions(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    page: PageParams,
) -> Result<Page<StudentSubmission>, AssessmentError> {
    let opened = open(conn, actor, assignment_id).await?;
    policy::authorize(actor, Action::ViewSubmissions(opened.access))?;
    require_submissions(&opened)?;
    let rows = repo::list_students(
        conn,
        opened.course_id,
        assignment_id,
        page.after,
        i64::from(page.limit) + 1,
    )
    .await?;
    Ok(Page::from_rows(rows, page.limit, |row| row.student_id))
}

/// A page of the submitted attempts of one student, newest first. Staff and admins. No draft is
/// read.
pub async fn student_attempts(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    student_id: Uuid,
    page: PageParams,
) -> Result<Page<Attempt>, AssessmentError> {
    let opened = open(conn, actor, assignment_id).await?;
    policy::authorize(actor, Action::ViewSubmissions(opened.access))?;
    require_submissions(&opened)?;
    if !repo::is_student_or_has_attempts(conn, opened.course_id, assignment_id, student_id).await? {
        return Err(AssessmentError::StudentNotFound);
    }
    let rows = repo::list_attempts(
        conn,
        assignment_id,
        student_id,
        page.after,
        i64::from(page.limit) + 1,
    )
    .await?;
    let mut page = Page::from_rows(rows, page.limit, |attempt| attempt.id);
    // The scores of a quiz are staff's: the student's own reads never fill them (ADR-0050).
    let ids: Vec<Uuid> = page.items.iter().map(|attempt| attempt.id).collect();
    for (id, scores) in repo::auto_scores_of(conn, &ids).await? {
        if let Some(attempt) = page.items.iter_mut().find(|attempt| attempt.id == id) {
            attempt.auto_scores = Some(scores);
        }
    }
    Ok(page)
}

// ---------------------------------------------------------------------------------------------
// Callbacks for `content` (and later `teaching`). The caller has authorized the actor and holds
// the lock of ADR-0035 point 12; none of these takes an `Actor`.
// ---------------------------------------------------------------------------------------------

/// True when the assignment has a submitted attempt.
pub async fn has_submitted_for_assignment(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<bool, AssessmentError> {
    Ok(repo::has_submitted_for_assignment(conn, assignment_id).await?)
}

/// True when an assignment in the module or its sub-modules has a submitted attempt.
pub async fn has_submitted_in_module(
    conn: &mut PgConnection,
    module_id: Uuid,
) -> Result<bool, AssessmentError> {
    Ok(repo::has_submitted_in_module(conn, module_id).await?)
}

/// True when an assignment of the course has a submitted attempt.
pub async fn has_submitted_in_course(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<bool, AssessmentError> {
    Ok(repo::has_submitted_in_course(conn, course_id).await?)
}

/// Deletes the drafts of the assignment, for the delete that follows in the same transaction.
/// Refuses with `HasSubmissions` instead of destroying an attempt. Returns how many drafts went.
pub async fn purge_assignment(
    conn: &mut PgConnection,
    assignment_id: Uuid,
) -> Result<u64, AssessmentError> {
    if repo::has_submitted_for_assignment(conn, assignment_id).await? {
        return Err(AssessmentError::HasSubmissions);
    }
    Ok(repo::purge_assignment(conn, assignment_id).await?)
}

/// Like [`purge_assignment`] for a module and its sub-modules.
pub async fn purge_module(
    conn: &mut PgConnection,
    module_id: Uuid,
) -> Result<u64, AssessmentError> {
    if repo::has_submitted_in_module(conn, module_id).await? {
        return Err(AssessmentError::HasSubmissions);
    }
    Ok(repo::purge_module(conn, module_id).await?)
}

/// Like [`purge_assignment`] for every assignment of a course.
pub async fn purge_course(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<u64, AssessmentError> {
    if repo::has_submitted_in_course(conn, course_id).await? {
        return Err(AssessmentError::HasSubmissions);
    }
    Ok(repo::purge_course(conn, course_id).await?)
}

#[cfg(test)]
mod tests;
