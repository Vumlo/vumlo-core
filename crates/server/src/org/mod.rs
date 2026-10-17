//! Terms, groups, subgroups and the installation record (ADR-0031, ADR-0032).

mod model;
mod repo;
mod routes;
mod service;

pub use model::{
    Group, GroupMember, MemberProblem, ModelError, OrgName, Subgroup, Term, TermDates, TermUpdate,
};
pub use routes::router;
pub use service::{OrgError, record_mode};
