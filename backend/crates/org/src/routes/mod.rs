//! HTTP edge of `org`: extractors, DTO mapping, OpenAPI annotations. Writes open the transaction,
//! call exactly one service function and commit; reads use a pooled connection.

mod groups;
mod institution;
mod subgroups;
mod terms;
mod unit_roles;
mod units;

use access::ActorSource;
use api_types::{FieldError, GroupMember, Page, PageQuery};
use axum::extract::FromRef;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::model::{self, MemberProblem, ModelError};
use super::service::OrgError;
use crate::OrgState;
use common::ApiError;
use common::{self, PageParams};

pub fn router<S>() -> OpenApiRouter<S>
where
    S: ActorSource + Clone + 'static,
    OrgState: FromRef<S>,
{
    OpenApiRouter::new()
        .routes(routes!(institution::get_institution))
        .routes(routes!(institution::get_admin_institution))
        .routes(routes!(institution::set_app_downloads))
        .routes(routes!(terms::create_term, terms::list_terms))
        .routes(routes!(
            terms::get_term,
            terms::update_term,
            terms::delete_term
        ))
        .routes(routes!(groups::create_group, groups::list_groups))
        .routes(routes!(
            groups::get_group,
            groups::rename_group,
            groups::delete_group
        ))
        .routes(routes!(
            groups::list_group_members,
            groups::add_group_members,
            groups::replace_group_members
        ))
        .routes(routes!(groups::remove_group_members))
        .routes(routes!(groups::remove_group_member))
        .routes(routes!(
            subgroups::create_subgroup,
            subgroups::list_subgroups
        ))
        .routes(routes!(
            subgroups::rename_subgroup,
            subgroups::delete_subgroup
        ))
        .routes(routes!(
            subgroups::list_subgroup_members,
            subgroups::add_subgroup_members
        ))
        .routes(routes!(subgroups::remove_subgroup_member))
        .routes(routes!(units::create_unit, units::list_units))
        .routes(routes!(units::update_unit, units::delete_unit))
        .routes(routes!(units::list_unit_roles))
        .routes(routes!(units::put_unit_role, units::delete_unit_role))
        .routes(routes!(units::get_unit_candidates))
        .routes(routes!(unit_roles::get_unit_role_abilities))
        .routes(routes!(unit_roles::put_unit_role_abilities))
}

impl From<OrgError> for ApiError {
    fn from(error: OrgError) -> Self {
        match error {
            OrgError::Forbidden(policy) => policy.into(),
            OrgError::Invalid(ModelError::Name) => Self::invalid_field("name", error.to_string()),
            OrgError::Invalid(ModelError::Dates | ModelError::EndsOnRange) => {
                Self::invalid_field("ends_on", error.to_string())
            }
            OrgError::Invalid(ModelError::StartsOnRange) => {
                Self::invalid_field("starts_on", error.to_string())
            }
            OrgError::Invalid(ModelError::DownloadUrl | ModelError::DownloadDuplicate) => {
                Self::invalid_field("downloads", error.to_string())
            }
            OrgError::TermNotFound
            | OrgError::UnitNotFound
            | OrgError::GroupNotFound
            | OrgError::SubgroupNotFound
            | OrgError::MemberNotFound => Self::NotFound,
            OrgError::TermNameTaken => Self::Conflict("a term with this name already exists"),
            OrgError::GroupNameTaken => Self::Conflict("a group with this name already exists"),
            OrgError::SubgroupNameTaken => {
                Self::Conflict("a subgroup with this name already exists in the group")
            }
            OrgError::UnitNameTaken => {
                Self::Conflict("a unit with this name already exists under the parent")
            }
            OrgError::UnitInUse => Self::Conflict("the unit has subunits or courses"),
            OrgError::UnitCycle | OrgError::ParentNotValid => {
                Self::invalid_field("parent_id", error.to_string())
            }
            OrgError::RoleNotAllowed => Self::invalid_field("role", error.to_string()),
            OrgError::QueryTooShort => Self::invalid_field("q", "needs at least two characters"),
            OrgError::TermInUse => Self::Conflict("a course uses this term"),
            OrgError::GroupInUse => Self::Conflict("a course uses this group"),
            OrgError::SubgroupInUse => Self::Conflict("a course uses this subgroup"),
            OrgError::Members(refused) => Self::Unprocessable {
                detail: "the request has invalid fields".to_owned(),
                errors: refused
                    .into_iter()
                    .map(|(position, problem)| FieldError {
                        field: format!("user_ids[{position}]"),
                        message: match problem {
                            MemberProblem::NoSuchUser => "no such user",
                            MemberProblem::NotAStudent => "only students can join a group",
                            MemberProblem::NotInGroup => "is not a member of the group",
                        }
                        .to_owned(),
                    })
                    .collect(),
            },
            OrgError::Identity(e) => e.into(),
            OrgError::Events(e) => e.into(),
            OrgError::Db(e) => Self::from_db("org", &e),
            OrgError::ModeChanged { .. } => {
                tracing::error!(error = %error, "org: installation record error");
                Self::Internal
            }
        }
    }
}

fn page_params(query: &PageQuery) -> Result<PageParams, ApiError> {
    Ok(PageParams::new(query.cursor.as_deref(), query.limit)?)
}

fn api_page<T, U>(page: common::Page<T>, map: impl Fn(T) -> U) -> Page<U> {
    Page {
        items: page.items.into_iter().map(map).collect(),
        next_cursor: page.next.map(|id| id.to_string()),
    }
}

fn api_member(member: model::GroupMember) -> GroupMember {
    GroupMember {
        user_id: member.user_id,
        name: member.name,
        surname: member.surname,
    }
}
