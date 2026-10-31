//! Uploaded files (ADR-0069). The bytes travel as the raw body of `POST /files` and
//! `GET /files/{id}`; these are the query of the upload and the description that comes back.

use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// A stored file, as a document or a submission names it by `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FileRefDto {
    pub id: Uuid,
    /// The cleaned file name: no path, at most 200 characters.
    pub name: String,
    /// The content type found from the bytes, not the one the client claimed.
    pub mime: String,
    /// Bytes.
    pub size: i64,
}

/// The query of `POST /files`.
#[derive(Debug, Clone, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct UploadQuery {
    /// Whom the file is for: `item:{id}` (needs `EditContent`) or `assignment:{id}` (needs
    /// `SubmitWork`). The item or assignment must exist.
    #[serde(rename = "for")]
    #[garde(skip)]
    #[param(rename = "for")]
    pub owner: String,
    /// The file name as the client has it. It is cleaned before it is stored.
    #[garde(length(min = 1, max = 2000))]
    pub name: String,
}
