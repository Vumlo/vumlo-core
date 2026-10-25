//! The bottom layer: what every other crate may use and that names none of them (ADR-0065).

mod error;
mod pagination;
mod role;
mod text_enum;

pub use error::{ApiError, ValidJson, ValidQuery, problem_details};
pub use pagination::{InvalidCursor, Page, PageParams};
pub use role::Role;
