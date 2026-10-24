//! The bottom layer: what every other crate may use and that names none of them (ADR-0065).

mod role;
mod text_enum;

pub use role::Role;
