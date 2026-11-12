//! What needs several modules about items and modules: reading a form, changing and deleting an
//! item or a module, and the result of a quiz (ADR-0093). `content` does its part before and
//! after, and `assessment` and `gradebook` answer in between; none of them is asked from below.

mod routes;
mod service;

pub use routes::router;
pub use service::{delete_item, delete_module, get, result, update_item};
