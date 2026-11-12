//! Business rules. Every function that acts for a user authorizes first, takes the connection it
//! runs on and never begins or commits a transaction.
//!
//! A file the actor may not read does not exist as far as they can tell: an unknown id, a file of
//! a draft item, of someone else's submission and of nobody all give `NotFound` (ADR-0069).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use files::{File, FileVerdict, FilesError, Stored};
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{ItemReference, Owner, UsageOwner};
use super::repo;
use crate::assessment::{self, AssessmentError};
use crate::content::{self, ContentError, FileRef, distinct_ids};
use access::{AccessError, Actor};

#[derive(Debug, thiserror::Error)]
pub enum FileError {
    /// No such file, or the actor may not read it, or no such owner.
    #[error("file not found")]
    NotFound,
    /// The owner takes no more files now (an archived course).
    #[error("{0}")]
    Conflict(&'static str),
    #[error("the file is larger than the allowed size")]
    TooLarge,
    #[error("the file is empty")]
    Empty,
    #[error(transparent)]
    Forbidden(AccessError),
    #[error(transparent)]
    Files(FilesError),
    #[error(transparent)]
    Content(ContentError),
    #[error(transparent)]
    Assessment(AssessmentError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

impl From<FilesError> for FileError {
    fn from(error: FilesError) -> Self {
        match error {
            FilesError::Empty => Self::Empty,
            FilesError::TooLarge => Self::TooLarge,
            other => Self::Files(other),
        }
    }
}

impl From<ContentError> for FileError {
    fn from(error: ContentError) -> Self {
        match error {
            ContentError::ItemNotFound => Self::NotFound,
            ContentError::Forbidden(AccessError::Archived) => {
                Self::Conflict("the course is archived")
            }
            ContentError::Forbidden(denied) => Self::Forbidden(denied),
            other => Self::Content(other),
        }
    }
}

impl From<AssessmentError> for FileError {
    fn from(error: AssessmentError) -> Self {
        match error {
            AssessmentError::AssignmentNotFound => Self::NotFound,
            AssessmentError::Forbidden(AccessError::Archived) => {
                Self::Conflict("the course is archived")
            }
            AssessmentError::Forbidden(denied) => Self::Forbidden(denied),
            AssessmentError::TakesNoSubmissions => {
                Self::Conflict("this assignment takes no submissions")
            }
            AssessmentError::WindowClosed => Self::Conflict("the submission window is closed"),
            AssessmentError::AttemptLimit => Self::Conflict("the attempt limit is reached"),
            other => Self::Assessment(other),
        }
    }
}

/// May the actor upload a file for `owner`? Call it before reading the body. An owner they may not
/// see is `NotFound`, one they may see but not change is `Forbidden`, and an archived course is
/// `Conflict`. So is an assignment that takes no more submissions at `now` (ADR-0035).
pub async fn authorize_upload(
    conn: &mut PgConnection,
    actor: &Actor,
    owner: &Owner,
    now: DateTime<Utc>,
) -> Result<(), FileError> {
    match *owner {
        Owner::Item(item_id) => content::authorize_edit_item(conn, actor, item_id).await?,
        Owner::Assignment(assignment_id) => {
            assessment::authorize_submit(conn, actor, assignment_id, now).await?;
        }
    }
    Ok(())
}

/// Describes the bytes `files::put` stored, for `owner`. It authorizes again: the check before
/// the body was read may be minutes old. The file stays unreferenced until a document or
/// submission names it.
pub async fn register_upload(
    conn: &mut PgConnection,
    actor: &Actor,
    owner: &Owner,
    stored: &Stored,
    name: &str,
    mime: &str,
    now: DateTime<Utc>,
) -> Result<File, FileError> {
    authorize_upload(conn, actor, owner, now).await?;
    Ok(files::insert(conn, stored, actor.user_id(), name, mime).await?)
}

/// The file, if the actor may read it: if **any** row that references it grants access (an item's
/// published copy to whoever may view the item, its working copy to whoever may manage content, a
/// submission to its author and, once submitted, to staff), or, when nothing references it, if the
/// actor uploaded it. Every denial, "no such file" included, is `NotFound`.
pub async fn open_for_read(
    conn: &mut PgConnection,
    actor: &Actor,
    file_id: Uuid,
) -> Result<File, FileError> {
    let file = files::get(conn, file_id)
        .await?
        .ok_or(FileError::NotFound)?;
    let items = repo::item_references(conn, file_id).await?;
    let submissions = repo::submission_references(conn, file_id).await?;
    if items.is_empty() && submissions.is_empty() {
        return if file.uploaded_by == actor.user_id() {
            Ok(file)
        } else {
            Err(FileError::NotFound)
        };
    }
    for reference in items {
        if content::may_read_item_files(conn, actor, reference.item_id, reference.published).await?
        {
            return Ok(file);
        }
    }
    for submission_id in submissions {
        if assessment::may_read_submission_files(conn, actor, submission_id).await? {
            return Ok(file);
        }
    }
    Err(FileError::NotFound)
}

/// May a document or submission of `owner` name the files of `refs`? The answer is plain data
/// ([`FileVerdict`]) that the owner's `finish_*` turns into its own error, in the same
/// transaction, after this call.
///
/// The named files are locked first (`files::lock`, ascending id), and only then are the
/// references read. Whoever attaches a file locks it before checking, so two attaches of one
/// upload run one after the other: the second one reads the first one's reference and is refused.
/// The locks last until the caller's transaction ends, which is after its `finish_*` has written
/// the new references.
///
/// A file is usable if it already belongs to this owner (whoever else holds it too, as after
/// copying a course), or the actor uploaded it and nothing references it yet, or, for a
/// submission, only the actor's own earlier attempts on the same assignment hold it (a
/// resubmission). Anything else, an unknown id included, is unusable: a user cannot borrow the
/// file id of someone else's work. `unusable` is the first such file in the order of `refs`;
/// `not_image` the first file named by an image block whose stored type (the one the server
/// detected at upload, never the client's claim) is not an image. The caller has authorized the
/// write itself.
pub async fn check(
    conn: &mut PgConnection,
    user_id: Uuid,
    owner: &UsageOwner,
    refs: &[FileRef],
) -> Result<FileVerdict, FileError> {
    let ids = distinct_ids(refs);
    let rows: BTreeMap<Uuid, File> = files::lock(conn, &ids)
        .await?
        .into_iter()
        .map(|file| (file.id, file))
        .collect();
    let mut verdict = FileVerdict::default();
    for id in ids {
        let usable = match rows.get(&id) {
            Some(file) => may_name(conn, user_id, owner, file).await?,
            None => false,
        };
        if !usable {
            verdict.unusable = Some(id);
            break;
        }
    }
    verdict.not_image = refs
        .iter()
        .filter(|file| file.image)
        .find(|file| {
            rows.get(&file.id)
                .is_some_and(|row| !files::is_image(&row.mime))
        })
        .map(|file| file.id);
    verdict.names = rows.into_iter().map(|(id, file)| (id, file.name)).collect();
    Ok(verdict)
}

/// Rules 1 to 3 for one locked file.
async fn may_name(
    conn: &mut PgConnection,
    user_id: Uuid,
    owner: &UsageOwner,
    file: &File,
) -> Result<bool, FileError> {
    let items = repo::item_references(conn, file.id).await?;
    let submissions = repo::submission_references(conn, file.id).await?;
    if items.is_empty() && submissions.is_empty() {
        return Ok(file.uploaded_by == user_id);
    }
    let held_here = match *owner {
        UsageOwner::Item(item_id) => items.iter().any(|r| r.item_id == item_id),
        UsageOwner::Submission { id, .. } => submissions.contains(&id),
    };
    Ok(held_here || held_by_own_attempts(conn, user_id, owner, &items, &submissions).await?)
}

/// A resubmission starts from the earlier answer, so a new draft may name a file that only the
/// author's own earlier attempts on the same assignment hold. A file that another student's row,
/// another assignment's row or an item also holds is not theirs to carry over.
async fn held_by_own_attempts(
    conn: &mut PgConnection,
    user_id: Uuid,
    owner: &UsageOwner,
    items: &[ItemReference],
    submissions: &[Uuid],
) -> Result<bool, FileError> {
    let UsageOwner::Submission { assignment_id, .. } = *owner else {
        return Ok(false);
    };
    if !items.is_empty() {
        return Ok(false);
    }
    let own = repo::count_own_rows_on(conn, submissions, assignment_id, user_id).await?;
    Ok(usize::try_from(own).is_ok_and(|own| own == submissions.len()))
}

#[cfg(test)]
mod tests;
