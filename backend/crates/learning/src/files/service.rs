//! Business rules. Every function that acts for a user authorizes first, takes the connection it
//! runs on and never begins or commits a transaction.
//!
//! A file the actor may not read does not exist as far as they can tell: an unknown id, a file of
//! a draft item, of someone else's submission and of nobody all give `NotFound` (ADR-0069).

use files::{File, FilesError, Stored};
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{Owner, UsageOwner};
use super::repo;
use crate::assessment::{self, AssessmentError};
use crate::content::{self, ContentError};
use access::{AccessError, Actor};

#[derive(Debug, thiserror::Error)]
pub enum FileError {
    /// No such file, or the actor may not read it, or no such owner.
    #[error("file not found")]
    NotFound,
    /// The owner takes no more files now (an archived course).
    #[error("{0}")]
    Conflict(&'static str),
    /// The file does not exist, or belongs to someone else's item or submission, or was uploaded
    /// by another user and is not on this owner yet.
    #[error("the file {id} cannot be used here")]
    Unusable { id: Uuid },
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
            other => Self::Assessment(other),
        }
    }
}

/// May the actor upload a file for `owner`? Call it before reading the body. An owner they may not
/// see is `NotFound`, one they may see but not change is `Forbidden`, and an archived course is
/// `Conflict`.
pub async fn authorize_upload(
    conn: &mut PgConnection,
    actor: &Actor,
    owner: &Owner,
) -> Result<(), FileError> {
    match *owner {
        Owner::Item(item_id) => content::authorize_edit_item(conn, actor, item_id).await?,
        Owner::Assignment(assignment_id) => {
            assessment::authorize_submit(conn, actor, assignment_id).await?;
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
) -> Result<File, FileError> {
    authorize_upload(conn, actor, owner).await?;
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

/// May a document or submission of `owner` name each of `ids`? Yes if the file already belongs to
/// this owner (whoever else holds it too, as after copying a course), or the actor uploaded it and
/// nothing references it yet. Anything else, an unknown id
/// included, is `Unusable`: a user cannot borrow the file id of someone else's work. The caller has
/// authorized the write itself.
pub async fn ensure_usable(
    conn: &mut PgConnection,
    actor: &Actor,
    owner: &UsageOwner,
    ids: &[Uuid],
) -> Result<(), FileError> {
    for &id in ids {
        let file = files::get(conn, id)
            .await?
            .ok_or(FileError::Unusable { id })?;
        let holders: Vec<UsageOwner> = repo::item_references(conn, id)
            .await?
            .into_iter()
            .map(|reference| UsageOwner::Item(reference.item_id))
            .chain(
                repo::submission_references(conn, id)
                    .await?
                    .into_iter()
                    .map(UsageOwner::Submission),
            )
            .collect();
        let usable = if holders.is_empty() {
            file.uploaded_by == actor.user_id()
        } else {
            holders.contains(owner)
        };
        if !usable {
            return Err(FileError::Unusable { id });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
