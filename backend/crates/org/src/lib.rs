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
    Subgroup, Term, TermDates, TermUpdate, Unit, UnitCandidate, UnitRoleEntry, UnitUpdate,
};
pub use routes::router;
pub use service::{
    OrgError, add_group_members, apply_role_abilities, create_group, create_term, create_unit,
    delete_unit, drop_unit_roles, get_group, group_member_lines, group_options, groups_of_user,
    list_groups, list_terms, list_unit_roles, list_units, record_mode, remove_unit_role,
    role_abilities, set_unit_role, set_unit_role_abilities, terms_newest_first, unit_candidates,
    unit_grant, unit_role_abilities, update_unit, view_institution_settings,
};
pub use state::OrgState;
#[cfg(feature = "testing")]
pub use test_support::seed_group;
