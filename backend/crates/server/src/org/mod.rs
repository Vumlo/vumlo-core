//! Terms, groups, subgroups and the installation record (ADR-0031, ADR-0032).

mod model;
mod repo;
mod routes;
mod service;

pub use model::{
    DownloadUrl, Group, GroupMember, GroupOption, MemberLine, MemberProblem, ModelError, OrgName,
    Subgroup, Term, TermDates, TermUpdate,
};
pub use routes::router;
#[cfg(test)]
pub use service::seed_group;
pub use service::{
    OrgError, group_member_lines, group_options, groups_of_user, record_mode, terms_newest_first,
    view_institution_settings,
};
