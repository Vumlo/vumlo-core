//! The bottom layer: what every other crate may use and that names none of them (ADR-0065).

mod content_disposition;
mod error;
mod pagination;
mod role;
mod search;
mod text_enum;

pub use content_disposition::content_disposition;
pub use error::{ApiError, PassThrough, ValidJson, ValidQuery, problem_details};
pub use pagination::{InvalidCursor, Page, PageParams};
pub use role::Role;
pub use search::{CANDIDATE_LIMIT, CANDIDATE_MIN_CHARS, like_pattern};
