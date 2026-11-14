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

use std::collections::BTreeMap;

use ::files::FileVerdict;
use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{
    Attempt, AutoTotal, Draft, DueDraft, FormFacts, MySubmission, PublishedScore, QuizResult,
    QuizResultStart, Responses, StudentSubmission, Window,
};
use super::repo;
use access::Actor;
use access::{self, AccessError, Action};
use activity::{AutoGrade, DocumentErrors, EffectiveProfiles, FileRef, Kind, Stage, distinct_ids};
use common::{Page, PageParams};
use content::{self, Assignment, OnDeadline, OpenError, OpenedAssignment, form, quiz};
use events::{self, Event, EventsError};
use teaching::TeachingError;

#[derive(Debug, thiserror::Error)]
pub enum AssessmentError {
    #[error(transparent)]
    Forbidden(#[from] AccessError),
    /// No such assignment, or the actor may not see it.
    #[error("assignment not found")]
    AssignmentNotFound,
    /// No such submitted attempt (a draft does not count).
    #[error("submission not found")]
    SubmissionNotFound,
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
    /// A rule keeps the assignment closed to the student until they meet its conditions
    /// (ADR-0092).
    #[error("the assignment is locked until its conditions are met")]
    Locked,
    /// Another submit of the same student got the attempt number first.
    #[error("a concurrent submit took this attempt")]
    Concurrent,
    /// The scheduler found the assignment no longer set to auto-submit, or its deadline moved
    /// ahead, since it listed the draft (ADR-0091).
    #[error("the assignment is not due for an auto-submit")]
    NotDue,
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
    Teaching(TeachingError),
    #[error(transparent)]
    Events(#[from] EventsError),
    #[error(transparent)]
    Db(sqlx::Error),
}

impl AssessmentError {
    /// Whether the error says that this draft cannot be submitted as it is: the content is not
    /// valid or is blank, the attempts are used up, an exclusive option is gone, the student has
    /// left the course. The scheduler marks such a draft and does not try it again (ADR-0091). Any
    /// other error is a fault or a race, and the draft is tried on the next tick.
    #[must_use]
    pub fn refuses_submit(&self) -> bool {
        matches!(
            self,
            Self::Document(_)
                | Self::Empty
                | Self::AttemptLimit
                | Self::OptionTaken
                | Self::StudentNotFound
                | Self::TakesNoSubmissions
        )
    }
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
    if opened.assignment_type.capabilities().submissions {
        Ok(())
    } else {
        Err(AssessmentError::TakesNoSubmissions)
    }
}

/// An assignment that a rule keeps closed takes no work yet: no draft, no submit, no upload
/// (ADR-0092). Reading and discarding what the student already has stay possible.
fn require_unlocked(opened: &OpenedAssignment) -> Result<(), AssessmentError> {
    if opened.locked {
        Err(AssessmentError::Locked)
    } else {
        Ok(())
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

/// Whether the student, who has made `last` attempts, may make another.
fn within_attempts(assignment: &Assignment, last: i32) -> Result<(), AssessmentError> {
    if assignment
        .settings
        .max_attempts
        .is_some_and(|max| last >= max)
    {
        Err(AssessmentError::AttemptLimit)
    } else {
        Ok(())
    }
}

/// Whether a save or a submit is taken at `now`, and whether it is late.
fn open_window(assignment: &Assignment, now: DateTime<Utc>) -> Result<bool, AssessmentError> {
    match Window::at(&assignment.settings, now) {
        Window::Open { late } => Ok(late),
        Window::Closed => Err(AssessmentError::WindowClosed),
    }
}

/// A draft save that passed everything `assessment` can judge by itself, with the draft row locked
/// and the id it will keep: the state between [`begin_save_draft`] and [`finish_save_draft`].
#[derive(Debug)]
pub struct DraftSave {
    assignment: Assignment,
    assignment_id: Uuid,
    student_id: Uuid,
    id: Uuid,
    body: Map<String, Value>,
    files: Vec<FileRef>,
}

impl DraftSave {
    /// The row the draft is saved in: the owner of the files it names.
    pub fn submission_id(&self) -> Uuid {
        self.id
    }

    pub fn assignment_id(&self) -> Uuid {
        self.assignment_id
    }

    /// The files the content names.
    pub fn files(&self) -> &[FileRef] {
        &self.files
    }
}

/// First half of replacing the caller's draft with `content`; the last write wins and nothing is
/// versioned. Writes no event (ADR-0007). Authorizes, validates and lists the files the content
/// names; the flow (`learning::submissions::save_draft`) has them checked and calls
/// [`finish_save_draft`].
///
/// The assignment row is locked `FOR SHARE` before the settings are read from it, so a change of
/// the window made by a transaction that began first is seen, and one that comes later waits.
pub async fn begin_save_draft(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    content: &Value,
    profiles: &EffectiveProfiles,
    now: DateTime<Utc>,
) -> Result<DraftSave, AssessmentError> {
    let opened = open(conn, actor, assignment_id).await?;
    access::authorize(actor, Action::SubmitWork(opened.access))?;
    require_submissions(&opened)?;
    require_unlocked(&opened)?;
    let assignment = lock(conn, assignment_id).await?;
    open_window(&assignment, now)?;
    let valid = assignment
        .assignment_type
        .validate_submission(&assignment.config, content, Stage::Draft, profiles)
        .map_err(AssessmentError::Document)?;
    // The row is locked, so the id the files are checked against is the one the write keeps.
    let id = match repo::lock_draft(conn, assignment_id, actor.user_id()).await? {
        Some(draft) => draft.id,
        None => {
            // No row to lock: the first saves of this draft take turns, and the one that waited
            // looks again, because the one before it has written the row by now.
            repo::lock_first_draft(conn, assignment_id, actor.user_id()).await?;
            match repo::lock_draft(conn, assignment_id, actor.user_id()).await? {
                Some(draft) => draft.id,
                None => Uuid::now_v7(),
            }
        }
    };
    let files = file_refs(&assignment, valid.as_body())?;
    Ok(DraftSave {
        assignment,
        assignment_id,
        student_id: actor.user_id(),
        id,
        body: valid.as_body().clone(),
        files,
    })
}

/// Second half: raises what the check of the files said, then writes the draft and the rows that
/// name its files. The files were locked by the check, in the transaction that is still open.
pub async fn finish_save_draft(
    conn: &mut PgConnection,
    save: DraftSave,
    files: &FileVerdict,
) -> Result<Draft, AssessmentError> {
    let ids = judge_files(&save.assignment, &save.body, &save.files, files)?;
    let draft = repo::upsert_draft(
        conn,
        save.id,
        save.assignment_id,
        save.student_id,
        &save.body,
    )
    .await?;
    repo::replace_submission_files(conn, draft.id, &ids).await?;
    Ok(draft)
}

/// The caller's own work: the draft and a page of attempts, newest first.
pub async fn my_submission(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    page: PageParams,
) -> Result<MySubmission, AssessmentError> {
    let opened = open(conn, actor, assignment_id).await?;
    access::authorize(actor, Action::ViewOwnSubmission(opened.access))?;
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
    access::authorize(actor, Action::SubmitWork(opened.access))?;
    require_submissions(&opened)?;
    lock(conn, assignment_id).await?;
    repo::delete_draft(conn, assignment_id, actor.user_id()).await?;
    Ok(())
}

/// A submit that passed everything `assessment` can judge by itself, with the assignment row and
/// the draft row locked: the state between [`begin_submit`] or [`begin_auto_submit`] and
/// [`finish_submit`]. The draft row becomes the attempt in place.
#[derive(Debug)]
pub struct SubmitStart {
    assignment: Assignment,
    assignment_id: Uuid,
    student_id: Uuid,
    course_id: Uuid,
    draft_id: Uuid,
    last: i32,
    body: Map<String, Value>,
    submitted_at: DateTime<Utc>,
    is_late: bool,
    files: Vec<FileRef>,
}

impl SubmitStart {
    /// The row that becomes the attempt: the owner of the files it names.
    pub fn submission_id(&self) -> Uuid {
        self.draft_id
    }

    pub fn assignment_id(&self) -> Uuid {
        self.assignment_id
    }

    /// Whose work it is, and so who must have uploaded a file that nothing holds yet.
    pub fn student_id(&self) -> Uuid {
        self.student_id
    }

    /// The files the draft names.
    pub fn files(&self) -> &[FileRef] {
        &self.files
    }
}

/// First half of turning the caller's draft into their next attempt: everything up to the files.
///
/// Order: the assignment row is locked `FOR SHARE` and the window is judged on the settings in it;
/// then the draft row is locked `FOR UPDATE`, so a second submit of the same draft waits and then
/// finds no draft; then, in a later statement, the attempts are counted. The flow
/// (`learning::submissions::submit`) has the files checked, then [`finish_submit`] writes the
/// attempt and emits `submission.submitted`.
pub async fn begin_submit(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    profiles: &EffectiveProfiles,
    now: DateTime<Utc>,
) -> Result<SubmitStart, AssessmentError> {
    let opened = open(conn, actor, assignment_id).await?;
    access::authorize(actor, Action::SubmitWork(opened.access))?;
    require_submissions(&opened)?;
    require_unlocked(&opened)?;
    begin_submit_for(
        conn,
        actor.user_id(),
        assignment_id,
        opened.course_id,
        profiles,
        When::Student(now),
    )
    .await
}

/// First half of submitting the saved draft of a student whose assignment has
/// `on_deadline = auto_submit` and whose deadline has passed, as the scheduler's act and not the
/// student's (ADR-0091). Nothing here authorizes anyone: the draft is the student's own work,
/// saved by them, and the assignment's settings are the ones the teacher chose. Every rule of a
/// submit still holds: the student must be a current student, the draft must pass the document and
/// form rules, the files it names are checked again by the flow, and the attempt limit counts. The
/// attempt is stamped with `due_at`, and is never late. An access rule does not stop it: the draft
/// was saved while the assignment was open (ADR-0092).
///
/// `AssessmentError::refuses_submit` says which errors mean "this draft cannot be submitted";
/// `NoDraft` and `NotDue` mean the world changed since the draft was listed.
pub async fn begin_auto_submit(
    conn: &mut PgConnection,
    due: &DueDraft,
    profiles: &EffectiveProfiles,
    now: DateTime<Utc>,
) -> Result<SubmitStart, AssessmentError> {
    if repo::students_among(conn, due.course_id, &[due.student_id])
        .await?
        .is_empty()
    {
        return Err(AssessmentError::StudentNotFound);
    }
    begin_submit_for(
        conn,
        due.student_id,
        due.assignment_id,
        due.course_id,
        profiles,
        When::Deadline(now),
    )
    .await
}

/// The drafts whose assignment auto-submits and whose deadline passed before `now`, the oldest
/// deadline first, at most `limit` (ADR-0091). Only a draft that existed at `due_at` is listed: a
/// student who starts one in the late window is writing it, not leaving it behind. A draft that
/// was tried and refused is not listed again.
pub async fn due_drafts(
    conn: &mut PgConnection,
    now: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<DueDraft>, AssessmentError> {
    Ok(repo::due_drafts(conn, now, limit).await?)
}

/// Records that the scheduler tried the draft and the submit refused it, so it is kept as the
/// student left it and not tried again (ADR-0091).
pub async fn skip_auto_submit(
    conn: &mut PgConnection,
    due: &DueDraft,
    now: DateTime<Utc>,
) -> Result<(), AssessmentError> {
    repo::mark_auto_submit_skipped(conn, due.assignment_id, due.student_id, now).await?;
    Ok(())
}

/// The sum of a submitted attempt that was scored in full, or `None`: a question is left for a
/// person, or there is no such attempt. The draft grade of a quiz comes from it (ADR-0093). It
/// authorizes nothing: it serves the subscriber of the submit, never a request.
pub async fn auto_total(
    conn: &mut PgConnection,
    submission_id: Uuid,
) -> Result<Option<AutoTotal>, AssessmentError> {
    Ok(repo::auto_total(conn, submission_id).await?)
}

/// Who turns the draft into an attempt, and when.
enum When {
    /// The student, at `now`: the window must be open, and the attempt is late after `due_at`.
    Student(DateTime<Utc>),
    /// The scheduler, at `now`, for an auto-submit assignment whose `due_at` has passed.
    Deadline(DateTime<Utc>),
}

/// The first half of the submit, after whoever asked has been let in. The order of locks is the
/// one in the doc of [`begin_submit`], for both callers, so the scheduler and a student racing for
/// one draft end with one attempt.
async fn begin_submit_for(
    conn: &mut PgConnection,
    student_id: Uuid,
    assignment_id: Uuid,
    course_id: Uuid,
    profiles: &EffectiveProfiles,
    when: When,
) -> Result<SubmitStart, AssessmentError> {
    let assignment = lock(conn, assignment_id).await?;
    if !assignment.assignment_type.capabilities().submissions {
        return Err(AssessmentError::TakesNoSubmissions);
    }
    let (submitted_at, is_late) = match when {
        When::Student(now) => (now, open_window(&assignment, now)?),
        // Judged on the row as it is under the lock: a teacher may have moved the deadline or
        // switched the setting since the draft was listed.
        When::Deadline(now) => {
            match (assignment.settings.on_deadline, assignment.settings.due_at) {
                (Some(OnDeadline::AutoSubmit), Some(due_at)) if due_at <= now => (due_at, false),
                _ => return Err(AssessmentError::NotDue),
            }
        }
    };
    let draft = repo::lock_draft(conn, assignment_id, student_id)
        .await?
        .ok_or(AssessmentError::NoDraft)?;
    let last = repo::last_attempt_number(conn, assignment_id, student_id).await?;
    within_attempts(&assignment, last)?;
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
        course_id,
        student_id,
        valid.as_body(),
    )
    .await?;
    let files = file_refs(&assignment, valid.as_body())?;
    Ok(SubmitStart {
        assignment,
        assignment_id,
        student_id,
        course_id,
        draft_id: draft.id,
        last,
        body: valid.as_body().clone(),
        submitted_at,
        is_late,
        files,
    })
}

/// Second half: raises what the check of the files said (the files are checked again here, so a
/// draft whose file is gone is refused, not submitted with a dangling reference), scores what the
/// server can score, turns the draft into the attempt, names its files and emits the events.
pub async fn finish_submit(
    conn: &mut PgConnection,
    start: SubmitStart,
    files: &FileVerdict,
) -> Result<Attempt, AssessmentError> {
    let SubmitStart {
        assignment,
        assignment_id,
        student_id,
        course_id,
        draft_id,
        last,
        body,
        submitted_at,
        is_late,
        files: refs,
    } = start;
    let ids = judge_files(&assignment, &body, &refs, files)?;
    let auto = assignment
        .assignment_type
        .auto_grade(&assignment.config, &body);
    let scores = auto.as_ref().map(AutoGrade::scores_json);
    let total = auto.filter(|auto| auto.complete).map(|auto| auto.total);
    let attempt = repo::submit_draft(
        conn,
        draft_id,
        last + 1,
        submitted_at,
        is_late,
        scores.as_ref(),
        total,
    )
    .await?;
    repo::replace_submission_files(conn, attempt.id, &ids).await?;
    events::emit(
        conn,
        Event::SubmissionSubmitted {
            assignment_id,
            student_id,
            course_id,
            submission_id: attempt.id,
        },
    )
    .await?;
    if assignment.assignment_type.id() == quiz::ID {
        events::emit(
            conn,
            Event::QuizAttemptSubmitted {
                assignment_id,
                student_id,
                course_id,
                submission_id: attempt.id,
            },
        )
        .await?;
    }
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

/// The files a submission names: the listed ones of a `file` hand-in plus the images and files that
/// documents in the content name (a text answer, a long-text quiz answer, a comment). The content
/// passed the validator, so the same parser reads it; if it ever does not, the refusal names the
/// path rather than leaving a reference unchecked.
fn file_refs(
    assignment: &Assignment,
    body: &Map<String, Value>,
) -> Result<Vec<FileRef>, AssessmentError> {
    assignment
        .assignment_type
        .submission_file_refs(&assignment.config, body)
        .map_err(AssessmentError::Document)
}

/// What the check of the files (`learning::files::check`, ADR-0069) said about `refs`, as the
/// error of this write, or the ids to attach. The name of each file the content lists must also be
/// one the type takes (a `file` hand-in; the other types list none and take any).
///
/// Ownership is judged first, so a file of someone else and an unknown id give one message, and
/// the extension rule never tells whether a foreign id exists.
fn judge_files(
    assignment: &Assignment,
    body: &Map<String, Value>,
    refs: &[FileRef],
    verdict: &FileVerdict,
) -> Result<Vec<Uuid>, AssessmentError> {
    let path = |id: Uuid| {
        refs.iter()
            .find(|file| file.id == id)
            .map_or("files", |file| &file.path)
    };
    if let Some(id) = verdict.unusable {
        return Err(unusable(path(id)));
    }
    if let Some(id) = verdict.not_image {
        return Err(AssessmentError::Document(DocumentErrors::one(
            path(id),
            "an image block needs a PNG, JPEG, GIF or WebP file",
        )));
    }
    // The check names every file it judged. One it did not judge (an empty verdict handed to a
    // write that names files) is refused, so a write finished without the check attaches nothing.
    if let Some(file) = refs
        .iter()
        .find(|file| !verdict.names.contains_key(&file.id))
    {
        return Err(unusable(&file.path));
    }
    let kind = assignment.assignment_type;
    let listed = kind.claims(&assignment.config, body).files;
    let mut names = BTreeMap::new();
    for (index, &id) in listed.iter().enumerate() {
        let name = verdict
            .names
            .get(&id)
            .ok_or_else(|| unusable(&format!("files[{index}]")))?;
        names.insert(id, name.clone());
    }
    kind.check_file_names(&assignment.config, body, &names)
        .map_err(AssessmentError::Document)?;
    Ok(distinct_ids(refs))
}

fn unusable(path: &str) -> AssessmentError {
    AssessmentError::Document(DocumentErrors::one(path, "is not a file you can use here"))
}

/// The facts a form's config shows besides what was configured: which fields have answers and who
/// holds each exclusive option (ADR-0051 point 5). The caller has authorized the read.
pub async fn form_facts(
    conn: &mut PgConnection,
    assignment_id: Uuid,
    assignment_type: Kind,
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

/// First half of the result of a quiz: authorizes the caller as a student who may see their
/// grades, and locks the assignment so that the config does not change under the result. The
/// flow (`learning::items::result`) then reads the published grade of the caller from the
/// gradebook and calls [`finish_quiz_result`] (ADR-0050 point 8).
pub async fn begin_quiz_result(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
) -> Result<QuizResultStart, AssessmentError> {
    let opened = open(conn, actor, assignment_id).await?;
    access::authorize(actor, Action::ViewOwnGrades(opened.access))?;
    if opened.assignment_type.id() != quiz::ID {
        return Err(AssessmentError::NothingToShow);
    }
    let assignment = lock(conn, assignment_id).await?;
    Ok(QuizResultStart {
        assignment_id,
        assignment,
    })
}

/// The caller's result of a quiz: their latest attempt, the server's score of each question and the
/// published grade. Only once a score is published; before that there is nothing to show
/// (ADR-0050 point 8). `published` is what the gradebook published for the caller, if anything.
pub async fn finish_quiz_result(
    conn: &mut PgConnection,
    actor: &Actor,
    start: QuizResultStart,
    published: Option<PublishedScore>,
) -> Result<QuizResult, AssessmentError> {
    let QuizResultStart {
        assignment_id,
        assignment,
    } = start;
    let published = published.ok_or(AssessmentError::NothingToShow)?;
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
    access::authorize(actor, Action::ViewSubmissions(opened.access))?;
    if opened.assignment_type.id() != form::ID {
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
    access::authorize(actor, Action::ViewSubmissions(opened.access))?;
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
    access::authorize(actor, Action::ViewSubmissions(opened.access))?;
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

/// May the actor save work to the assignment at `now` (`SubmitWork`)? An assignment they may not
/// see is `AssignmentNotFound`; an archived course is `Forbidden(Archived)`. After that the answer
/// is the one a save would give, plus the attempt limit, which a save does not look at but after
/// which nothing can be submitted: a type that takes no submissions, a closed window and the
/// limit are conflicts. For the upload of a file that the submission will name.
pub async fn authorize_submit(
    conn: &mut PgConnection,
    actor: &Actor,
    assignment_id: Uuid,
    now: DateTime<Utc>,
) -> Result<(), AssessmentError> {
    let opened = open(conn, actor, assignment_id).await?;
    access::authorize(actor, Action::SubmitWork(opened.access))?;
    require_submissions(&opened)?;
    require_unlocked(&opened)?;
    let assignment = lock(conn, assignment_id).await?;
    open_window(&assignment, now)?;
    let last = repo::last_attempt_number(conn, assignment_id, actor.user_id()).await?;
    within_attempts(&assignment, last)
}

/// May the actor read the files of submission `submission_id` (ADR-0069)? Its author may, draft
/// or attempt (`ViewOwnSubmission`); staff may read a submitted attempt (`ViewSubmissions`), never
/// a draft. Anything else is `false`, never an error.
pub async fn may_read_submission_files(
    conn: &mut PgConnection,
    actor: &Actor,
    submission_id: Uuid,
) -> Result<bool, AssessmentError> {
    let Some((assignment_id, author)) = repo::assignment_and_author(conn, submission_id).await?
    else {
        return Ok(false);
    };
    if author == actor.user_id() {
        let opened = match open(conn, actor, assignment_id).await {
            Ok(opened) => opened,
            Err(AssessmentError::AssignmentNotFound) => return Ok(false),
            Err(other) => return Err(other),
        };
        match access::authorize(actor, Action::ViewOwnSubmission(opened.access)) {
            Ok(()) => return Ok(true),
            Err(AccessError::Denied) => {}
            Err(other) => return Err(other.into()),
        }
    }
    match authorize_view_submission(conn, actor, submission_id).await {
        Ok(()) => Ok(true),
        Err(
            AssessmentError::SubmissionNotFound
            | AssessmentError::AssignmentNotFound
            | AssessmentError::Forbidden(AccessError::Denied),
        ) => Ok(false),
        Err(other) => Err(other),
    }
}

/// May `actor` see the submitted attempt `submission_id` the way staff do (`ViewSubmissions` in its
/// course)? Anything they may not see, a draft included, is `404`. The caller then reads whatever
/// else hangs on the attempt, such as the reports of plugins.
pub async fn authorize_view_submission(
    conn: &mut PgConnection,
    actor: &Actor,
    submission_id: Uuid,
) -> Result<(), AssessmentError> {
    let assignment_id = repo::assignment_of_submitted(conn, submission_id)
        .await?
        .ok_or(AssessmentError::SubmissionNotFound)?;
    let opened = open(conn, actor, assignment_id).await?;
    access::authorize(actor, Action::ViewSubmissions(opened.access))?;
    Ok(())
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

/// The students who submitted an attempt in the course. The caller has authorized (it asks for
/// the roster page of staff).
pub async fn students_with_attempts(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<Vec<Uuid>, AssessmentError> {
    Ok(repo::students_with_attempts(conn, course_id).await?)
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
