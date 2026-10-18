//! Cursor pagination (ADR-0018), shared by every module that lists. It belongs to none of them.

use uuid::Uuid;

use crate::error::ApiError;

const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 100;

/// A cursor that is not the id of an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("is not a valid cursor")]
pub struct InvalidCursor;

impl From<InvalidCursor> for ApiError {
    fn from(error: InvalidCursor) -> Self {
        Self::invalid_field("cursor", error.to_string())
    }
}

/// Which page of a list to read (ADR-0018). The cursor is the id of the last item seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageParams {
    pub after: Option<Uuid>,
    pub limit: u32,
}

impl PageParams {
    pub fn new(cursor: Option<&str>, limit: Option<u32>) -> Result<Self, InvalidCursor> {
        let after = cursor
            .map(|raw| Uuid::parse_str(raw).map_err(|_| InvalidCursor))
            .transpose()?;
        Ok(Self {
            after,
            limit: limit.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// The cursor for the next page, `None` on the last one.
    pub next: Option<Uuid>,
}

impl<T> Page<T> {
    /// Builds a page from rows fetched with `limit + 1`: the extra row only proves there is more.
    pub fn from_rows(mut rows: Vec<T>, limit: u32, id: impl Fn(&T) -> Uuid) -> Self {
        let limit = limit as usize;
        let next = (rows.len() > limit).then(|| {
            rows.truncate(limit);
            rows.last().map(&id)
        });
        Self {
            items: rows,
            next: next.flatten(),
        }
    }
}

#[cfg(test)]
mod tests;
