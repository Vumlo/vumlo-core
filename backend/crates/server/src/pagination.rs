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
mod tests {
    use super::*;

    #[test]
    fn page_params_default_clamp_and_parse_the_cursor() {
        let default = PageParams::new(None, None).unwrap();
        assert_eq!((default.after, default.limit), (None, 50));
        assert_eq!(PageParams::new(None, Some(1000)).unwrap().limit, 100);
        assert_eq!(PageParams::new(None, Some(0)).unwrap().limit, 1);
        let id = Uuid::now_v7();
        assert_eq!(
            PageParams::new(Some(&id.to_string()), Some(5))
                .unwrap()
                .after,
            Some(id)
        );
        assert_eq!(PageParams::new(Some("junk"), None), Err(InvalidCursor));
    }

    #[test]
    fn page_from_rows_reports_a_next_cursor_only_when_there_is_more() {
        let ids: Vec<Uuid> = (0..3).map(|_| Uuid::now_v7()).collect();
        let full = Page::from_rows(ids.clone(), 2, |id| *id);
        assert_eq!(full.items, ids[..2]);
        assert_eq!(full.next, Some(ids[1]));
        let last = Page::from_rows(ids[..2].to_vec(), 2, |id| *id);
        assert_eq!(last.items.len(), 2);
        assert_eq!(last.next, None);
        assert_eq!(Page::from_rows(Vec::<Uuid>::new(), 2, |id| *id).next, None);
    }

    #[test]
    fn a_bad_cursor_is_a_422_on_the_cursor_field() {
        use axum::response::IntoResponse;
        let response = ApiError::from(InvalidCursor).into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::UNPROCESSABLE_ENTITY
        );
    }
}
