//! Terms, groups, subgroups and the installation record (ADR-0031, ADR-0032).

mod model;
mod repo;
mod routes;
mod service;
mod state;
#[cfg(feature = "testing")]
mod test_support;
#[cfg(test)]
mod tests;

pub use model::{
    DownloadUrl, Group, GroupMember, GroupOption, MemberLine, MemberProblem, ModelError, OrgName,
    Subgroup, Term, TermDates, TermUpdate, Unit, UnitUpdate,
};
pub use routes::router;
pub use service::{
    OrgError, add_group_members, create_group, create_term, create_unit, delete_unit, get_group,
    group_member_lines, group_options, groups_of_user, list_groups, list_terms, list_units,
    record_mode, terms_newest_first, unit_grant, update_unit, view_institution_settings,
};
pub use state::OrgState;
#[cfg(feature = "testing")]
pub use test_support::seed_group;
