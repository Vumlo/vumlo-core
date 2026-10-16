//! The installation record: the institution mode the first start recorded (ADR-0031). Terms and
//! groups (ADR-0001) are not built yet; there are no endpoints, so this module has no router.

mod repo;
mod service;

pub use service::{OrgError, record_mode};
