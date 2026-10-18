//! HTTP edge of `org`: extractors, DTO mapping, OpenAPI annotations. Writes open the transaction,
//! call exactly one service function and commit; reads use a pooled connection.

mod groups;
mod institution;
mod reports;
mod subgroups;
mod terms;

use api_types::{FieldError, GroupMember, Page, PageQuery};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::model::{self, MemberProblem, ModelError};
use super::service::OrgError;
use crate::app::AppState;
use crate::error::ApiError;
use crate::pagination::{self, PageParams};

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(institution::get_institution))
        .routes(routes!(institution::get_admin_institution))
        .routes(routes!(reports::get_group_overview))
        .routes(routes!(reports::get_term_stats))
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
            OrgError::TermNotFound
            | OrgError::GroupNotFound
            | OrgError::SubgroupNotFound
            | OrgError::MemberNotFound => Self::NotFound,
            OrgError::TermNameTaken => Self::Conflict("a term with this name already exists"),
            OrgError::GroupNameTaken => Self::Conflict("a group with this name already exists"),
            OrgError::SubgroupNameTaken => {
                Self::Conflict("a subgroup with this name already exists in the group")
            }
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
            OrgError::Teaching(e) => (*e).into(),
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

fn api_page<T, U>(page: pagination::Page<T>, map: impl Fn(T) -> U) -> Page<U> {
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
