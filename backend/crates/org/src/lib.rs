//! Terms, groups, subgroups and the installation record (ADR-0031, ADR-0032).

mod model;
mod repo;
mod routes;
mod service;
mod state;
#[cfg(feature = "testing")]
mod test_support;

pub use model::{
    DownloadUrl, Group, GroupMember, GroupOption, MemberLine, MemberProblem, ModelError, OrgName,
    Subgroup, Term, TermDates, TermUpdate,
};
pub use routes::router;
pub use service::{
    OrgError, group_member_lines, group_options, groups_of_user, record_mode, terms_newest_first,
    view_institution_settings,
};
pub use state::OrgState;
#[cfg(feature = "testing")]
pub use test_support::seed_group;
