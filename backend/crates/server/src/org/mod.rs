//! Terms, groups, subgroups and the installation record (ADR-0031, ADR-0032).

mod model;
mod repo;
mod routes;
mod service;

pub use model::{
    Group, GroupMember, GroupOption, GroupOverview, MemberLine, MemberProblem, ModelError, OrgName,
    Subgroup, Term, TermDates, TermUpdate, TermUsage,
};
pub use routes::router;
#[cfg(test)]
pub use service::seed_group;
pub use service::{
    OrgError, group_options, groups_of_user, record_mode, view_institution_settings,
};
