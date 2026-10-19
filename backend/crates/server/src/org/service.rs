//! Business rules. Every function that acts for a user authorizes first, takes the connection it
//! runs on and never begins or commits a transaction. Every write below a group takes the group
//! row lock first (ADR-0032).

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{
    DownloadUrl, Group, GroupMember, GroupOption, GroupOverview, MemberProblem, ModelError,
    OrgName, Subgroup, Term, TermDates, TermUpdate, TermUsage,
};
use super::repo;
use crate::actor::Actor;
use crate::events::{self, AuditEntry, EventsError};
use crate::identity::{self, IdentityError};
use crate::pagination::{Page, PageParams};
use crate::policy::{self, Action, PolicyError, group_role_allowed};
use crate::teaching::{self, TeachingError};
use vumlo_config::InstitutionMode;

#[derive(Debug, thiserror::Error)]
pub enum OrgError {
    #[error(transparent)]
    Forbidden(#[from] PolicyError),
    #[error(transparent)]
    Invalid(#[from] ModelError),
    #[error("term not found")]
    TermNotFound,
    #[error("group not found")]
    GroupNotFound,
    #[error("subgroup not found")]
    SubgroupNotFound,
    #[error("member not found")]
    MemberNotFound,
    #[error("a term with this name already exists")]
    TermNameTaken,
    #[error("a group with this name already exists")]
    GroupNameTaken,
    #[error("a subgroup with this name already exists in the group")]
    SubgroupNameTaken,
    #[error("a course uses this term")]
    TermInUse,
    #[error("a course uses this group")]
    GroupInUse,
    #[error("a course uses this subgroup")]
    SubgroupInUse,
    /// The positions (in the request's `user_ids`) that were refused, with the reason.
    #[error("some of the users cannot be members")]
    Members(Vec<(usize, MemberProblem)>),
    #[error(transparent)]
    Teaching(Box<TeachingError>),
    #[error(transparent)]
    Identity(#[from] IdentityError),
    #[error(transparent)]
    Events(#[from] EventsError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(
        "institution.mode is \"{}\" in the configuration, but this installation was set up as \"{}\"; the mode cannot be changed (ADR-0031)",
        configured.as_str(),
        recorded.as_str()
    )]
    ModeChanged {
        recorded: InstitutionMode,
        configured: InstitutionMode,
    },
}

/// Turns a unique violation on `constraint` into `taken`; any other error stays a database error.
fn name_taken(error: sqlx::Error, constraint: &str, taken: OrgError) -> OrgError {
    match &error {
        sqlx::Error::Database(db)
            if db.is_unique_violation() && db.constraint() == Some(constraint) =>
        {
            taken
        }
        _ => OrgError::Db(error),
    }
}

/// A course's link holds the group or the subgroup: `ON DELETE RESTRICT` reports 23001
/// (restrict_violation), not 23503. Deleting a group also fails this way when only one of its
/// subgroups is linked, because the cascade to the subgroup is refused.
fn in_use(error: sqlx::Error, in_use: OrgError) -> OrgError {
    match &error {
        sqlx::Error::Database(db) if db.code().as_deref() == Some("23001") => in_use,
        _ => OrgError::Db(error),
    }
}

async fn audit(
    conn: &mut PgConnection,
    actor: &Actor,
    action: &'static str,
    resource_type: &'static str,
    resource_id: Uuid,
    meta: serde_json::Value,
) -> Result<(), OrgError> {
    events::audit(
        conn,
        actor,
        AuditEntry {
            action,
            resource_type,
            resource_id,
            meta,
        },
    )
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Terms
// ---------------------------------------------------------------------------------------------

pub async fn create_term(
    conn: &mut PgConnection,
    actor: &Actor,
    name: &str,
    starts_on: NaiveDate,
    ends_on: NaiveDate,
) -> Result<Term, OrgError> {
    policy::authorize(actor, Action::ManageTerms)?;
    let name = OrgName::parse(name)?;
    let dates = TermDates::new(starts_on, ends_on)?;
    let term = repo::insert_term(
        conn,
        Uuid::now_v7(),
        name.as_str(),
        dates.starts_on,
        dates.ends_on,
    )
    .await
    .map_err(|e| name_taken(e, "terms_name_key", OrgError::TermNameTaken))?;
    audit(conn, actor, "term.create", "term", term.id, json!({})).await?;
    Ok(term)
}

pub async fn list_terms(
    conn: &mut PgConnection,
    actor: &Actor,
    page: PageParams,
) -> Result<Page<Term>, OrgError> {
    policy::authorize(actor, Action::ViewTerms)?;
    let rows = repo::list_terms(conn, page.after, i64::from(page.limit) + 1).await?;
    Ok(Page::from_rows(rows, page.limit, |term| term.id))
}

/// Authorizes the admin view of the institution settings (ADR-0044). The settings themselves come
/// from the configuration the server parsed at startup, so there is nothing to read.
pub fn view_institution_settings(actor: &Actor) -> Result<(), OrgError> {
    policy::authorize(actor, Action::ViewInstitutionSettings)?;
    Ok(())
}

/// The links to the desktop app's installers, for the public profile: no session, so no authorization.
pub async fn app_downloads(conn: &mut PgConnection) -> Result<Vec<(String, String)>, OrgError> {
    Ok(repo::list_app_downloads(conn).await?)
}

/// Sets the whole list of links: what is not in `links` is removed. Admin only; audited (ADR-0034).
pub async fn set_app_downloads(
    conn: &mut PgConnection,
    actor: &Actor,
    links: &[(&'static str, String)],
) -> Result<Vec<(String, String)>, OrgError> {
    policy::authorize(actor, Action::ManageAppDownloads)?;
    let mut clean: Vec<(String, String)> = Vec::with_capacity(links.len());
    for (platform, url) in links {
        if clean.iter().any(|(seen, _)| seen == platform) {
            return Err(ModelError::DownloadDuplicate.into());
        }
        clean.push((
            (*platform).to_owned(),
            DownloadUrl::parse(url)?.as_str().to_owned(),
        ));
    }
    repo::replace_app_downloads(conn, &clean).await?;
    // Which platforms have a link, never the addresses: they are public, but the log is for who did what.
    let platforms: Vec<&str> = clean
        .iter()
        .map(|(platform, _)| platform.as_str())
        .collect();
    // The institution has no row of its own: the audit entry names the installation by the nil id.
    audit(
        conn,
        actor,
        "institution.app_downloads.set",
        "institution",
        Uuid::nil(),
        json!({"platforms": platforms}),
    )
    .await?;
    Ok(repo::list_app_downloads(conn).await?)
}

pub async fn get_term(conn: &mut PgConnection, actor: &Actor, id: Uuid) -> Result<Term, OrgError> {
    policy::authorize(actor, Action::ViewTerms)?;
    repo::find_term(conn, id)
        .await?
        .ok_or(OrgError::TermNotFound)
}

/// Changes the fields that were sent. The dates are checked together after the merge, against the
/// row as it is while we hold its lock.
pub async fn update_term(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
    update: TermUpdate,
) -> Result<Term, OrgError> {
    policy::authorize(actor, Action::ManageTerms)?;
    let name = update.name.as_deref().map(OrgName::parse).transpose()?;
    let current = repo::lock_term(conn, id)
        .await?
        .ok_or(OrgError::TermNotFound)?;
    let dates = TermDates::new(
        update.starts_on.unwrap_or(current.starts_on),
        update.ends_on.unwrap_or(current.ends_on),
    )?;
    let term = repo::update_term(
        conn,
        id,
        name.as_ref().map_or(current.name.as_str(), OrgName::as_str),
        dates.starts_on,
        dates.ends_on,
    )
    .await
    .map_err(|e| name_taken(e, "terms_name_key", OrgError::TermNameTaken))?;
    audit(conn, actor, "term.update", "term", id, json!({})).await?;
    Ok(term)
}

/// Deletes a term no course uses. The foreign key from `teaching.courses` refuses the rest.
pub async fn delete_term(conn: &mut PgConnection, actor: &Actor, id: Uuid) -> Result<(), OrgError> {
    policy::authorize(actor, Action::ManageTerms)?;
    let deleted = repo::delete_term(conn, id).await.map_err(|e| match &e {
        // ON DELETE RESTRICT reports 23001 (restrict_violation), not 23503 like NO ACTION.
        sqlx::Error::Database(db)
            if db.code().as_deref() == Some("23001")
                && db.constraint() == Some("courses_term_id_fkey") =>
        {
            OrgError::TermInUse
        }
        _ => OrgError::Db(e),
    })?;
    if deleted == 0 {
        return Err(OrgError::TermNotFound);
    }
    audit(conn, actor, "term.delete", "term", id, json!({})).await
}

// ---------------------------------------------------------------------------------------------
// Groups
// ---------------------------------------------------------------------------------------------

pub async fn create_group(
    conn: &mut PgConnection,
    actor: &Actor,
    name: &str,
) -> Result<Group, OrgError> {
    policy::authorize(actor, Action::ManageGroups)?;
    let name = OrgName::parse(name)?;
    let group = repo::insert_group(conn, Uuid::now_v7(), name.as_str())
        .await
        .map_err(|e| name_taken(e, "groups_name_key", OrgError::GroupNameTaken))?;
    audit(conn, actor, "group.create", "group", group.id, json!({})).await?;
    Ok(group)
}

pub async fn list_groups(
    conn: &mut PgConnection,
    actor: &Actor,
    page: PageParams,
) -> Result<Page<Group>, OrgError> {
    policy::authorize(actor, Action::ViewGroups)?;
    let rows = repo::list_groups(conn, page.after, i64::from(page.limit) + 1).await?;
    Ok(Page::from_rows(rows, page.limit, |group| group.id))
}

pub async fn get_group(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<Group, OrgError> {
    policy::authorize(actor, Action::ViewGroups)?;
    repo::find_group(conn, id)
        .await?
        .ok_or(OrgError::GroupNotFound)
}

/// A group with its people and the courses that use it (ADR-0053). Admins only.
pub async fn group_overview(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<GroupOverview, OrgError> {
    policy::authorize(actor, Action::ViewGroups)?;
    repo::find_group(conn, id)
        .await?
        .ok_or(OrgError::GroupNotFound)?;
    Ok(GroupOverview {
        group_id: id,
        members: repo::group_member_lines(conn, id).await?,
        linked_courses: teaching::linked_courses_of_group(conn, id)
            .await
            .map_err(|e| OrgError::Teaching(Box::new(e)))?,
    })
}

/// The groups a person is in, by name. The read interface `identity` uses for a person's facts;
/// the caller has authorized the administrator.
pub async fn groups_of_user(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<Vec<(Uuid, String)>, OrgError> {
    Ok(repo::groups_of_user(conn, user_id).await?)
}

/// How many regular courses each term has, and whether it is current or over on `today`
/// (ADR-0053). Admins only.
pub async fn term_stats(
    conn: &mut PgConnection,
    actor: &Actor,
    today: NaiveDate,
) -> Result<Vec<TermUsage>, OrgError> {
    policy::authorize(actor, Action::ViewAdminReports)?;
    Ok(repo::term_usage(conn, today).await?)
}

pub async fn rename_group(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
    name: &str,
) -> Result<Group, OrgError> {
    policy::authorize(actor, Action::ManageGroups)?;
    let name = OrgName::parse(name)?;
    let group = repo::rename_group(conn, id, name.as_str())
        .await
        .map_err(|e| name_taken(e, "groups_name_key", OrgError::GroupNameTaken))?
        .ok_or(OrgError::GroupNotFound)?;
    audit(conn, actor, "group.rename", "group", id, json!({})).await?;
    Ok(group)
}

/// Deletes the group with its subgroups and members. The `DELETE` waits for a member write that
/// holds the group lock, and a write that waited for the delete finds no group.
pub async fn delete_group(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<(), OrgError> {
    policy::authorize(actor, Action::ManageGroups)?;
    let deleted = repo::delete_group(conn, id)
        .await
        .map_err(|e| in_use(e, OrgError::GroupInUse))?;
    if deleted == 0 {
        return Err(OrgError::GroupNotFound);
    }
    audit(conn, actor, "group.delete", "group", id, json!({})).await
}

/// The groups with their subgroups, ids and names only, after `after` (a group id), in id order.
///
/// Acts for no user: the caller (`teaching`, for those who may manage a course's roster) has
/// authorized its own action, as with `identity::find_user`. Groups stay admin-only to read
/// through this module's own endpoints (ADR-0033).
pub async fn group_options(
    conn: &mut PgConnection,
    page: PageParams,
) -> Result<Page<GroupOption>, OrgError> {
    let rows = repo::list_groups(conn, page.after, i64::from(page.limit) + 1).await?;
    let page = Page::from_rows(rows, page.limit, |group| group.id);
    let ids: Vec<Uuid> = page.items.iter().map(|group| group.id).collect();
    let mut subgroups = repo::subgroups_of(conn, &ids).await?;
    let items = page
        .items
        .into_iter()
        .map(|group| GroupOption {
            subgroups: subgroups
                .extract_if(.., |subgroup| subgroup.group_id == group.id)
                .collect(),
            id: group.id,
            name: group.name,
        })
        .collect();
    Ok(Page {
        items,
        next: page.next,
    })
}

pub async fn list_group_members(
    conn: &mut PgConnection,
    actor: &Actor,
    group_id: Uuid,
    page: PageParams,
) -> Result<Page<GroupMember>, OrgError> {
    policy::authorize(actor, Action::ViewGroups)?;
    repo::find_group(conn, group_id)
        .await?
        .ok_or(OrgError::GroupNotFound)?;
    let rows =
        repo::list_group_members(conn, group_id, page.after, i64::from(page.limit) + 1).await?;
    Ok(Page::from_rows(rows, page.limit, |member| member.user_id))
}

/// Refuses, with their positions, the entries of `user_ids` that are not members yet and are no
/// student (or no user). Someone who is a member already passes whatever their role is now.
async fn check_new_members(
    conn: &mut PgConnection,
    user_ids: &[Uuid],
    members: &HashSet<Uuid>,
) -> Result<(), OrgError> {
    let others: Vec<Uuid> = user_ids
        .iter()
        .filter(|id| !members.contains(id))
        .copied()
        .collect();
    let roles: HashMap<Uuid, identity::Role> = identity::find_users(conn, &others)
        .await?
        .into_iter()
        .map(|user| (user.id, user.role))
        .collect();
    let refused: Vec<(usize, MemberProblem)> = user_ids
        .iter()
        .enumerate()
        .filter(|(_, id)| !members.contains(id))
        .filter_map(|(position, id)| match roles.get(id) {
            None => Some((position, MemberProblem::NoSuchUser)),
            Some(role) if !group_role_allowed(*role) => {
                Some((position, MemberProblem::NotAStudent))
            }
            Some(_) => None,
        })
        .collect();
    if refused.is_empty() {
        Ok(())
    } else {
        Err(OrgError::Members(refused))
    }
}

/// Adds the users to the group and returns the ones who were not members yet. All or nothing:
/// every entry that is not a student (or not a user) is reported with its position, and then
/// nothing is added. Someone who is a member already passes whatever their role is now.
pub async fn add_group_members(
    conn: &mut PgConnection,
    actor: &Actor,
    group_id: Uuid,
    user_ids: &[Uuid],
) -> Result<Vec<Uuid>, OrgError> {
    policy::authorize(actor, Action::ManageGroups)?;
    repo::lock_group(conn, group_id)
        .await?
        .ok_or(OrgError::GroupNotFound)?;
    let members: HashSet<Uuid> = repo::group_members_among(conn, group_id, user_ids)
        .await?
        .into_iter()
        .collect();
    check_new_members(conn, user_ids, &members).await?;
    let added = repo::insert_group_members(conn, group_id, &distinct(user_ids)).await?;
    if !added.is_empty() {
        audit(
            conn,
            actor,
            "group.member.add",
            "group",
            group_id,
            json!({"user_ids": added}),
        )
        .await?;
    }
    Ok(added)
}

/// Makes `user_ids` exactly the members of the group (ADR-0037): adds the ones who are not members,
/// removes the members who are not listed, and returns both lists. All or nothing, like
/// [`add_group_members`]; the subgroup memberships of a removed member go with them. The rollover
/// of a class is one call.
pub async fn replace_group_members(
    conn: &mut PgConnection,
    actor: &Actor,
    group_id: Uuid,
    user_ids: &[Uuid],
) -> Result<(Vec<Uuid>, Vec<Uuid>), OrgError> {
    policy::authorize(actor, Action::ManageGroups)?;
    repo::lock_group(conn, group_id)
        .await?
        .ok_or(OrgError::GroupNotFound)?;
    let members: HashSet<Uuid> = repo::group_member_ids(conn, group_id)
        .await?
        .into_iter()
        .collect();
    check_new_members(conn, user_ids, &members).await?;
    let wanted: HashSet<Uuid> = user_ids.iter().copied().collect();
    let leaving: Vec<Uuid> = members.difference(&wanted).copied().collect();
    let removed = repo::delete_group_members(conn, group_id, &leaving).await?;
    let added = repo::insert_group_members(conn, group_id, &distinct(user_ids)).await?;
    if !added.is_empty() || !removed.is_empty() {
        audit(
            conn,
            actor,
            "group.members.replace",
            "group",
            group_id,
            json!({"added": added, "removed": removed}),
        )
        .await?;
    }
    Ok((added, removed))
}

/// Removes several users from the group and from its subgroups; the ones who are not members are
/// skipped. Returns the ones who were removed.
pub async fn remove_group_members(
    conn: &mut PgConnection,
    actor: &Actor,
    group_id: Uuid,
    user_ids: &[Uuid],
) -> Result<Vec<Uuid>, OrgError> {
    policy::authorize(actor, Action::ManageGroups)?;
    repo::lock_group(conn, group_id)
        .await?
        .ok_or(OrgError::GroupNotFound)?;
    let removed = repo::delete_group_members(conn, group_id, &distinct(user_ids)).await?;
    if !removed.is_empty() {
        audit(
            conn,
            actor,
            "group.members.remove",
            "group",
            group_id,
            json!({"user_ids": removed}),
        )
        .await?;
    }
    Ok(removed)
}

/// Removes the user from the group and, with them, from the group's subgroups.
pub async fn remove_group_member(
    conn: &mut PgConnection,
    actor: &Actor,
    group_id: Uuid,
    user_id: Uuid,
) -> Result<(), OrgError> {
    policy::authorize(actor, Action::ManageGroups)?;
    repo::lock_group(conn, group_id)
        .await?
        .ok_or(OrgError::GroupNotFound)?;
    if repo::delete_group_member(conn, group_id, user_id).await? == 0 {
        return Err(OrgError::MemberNotFound);
    }
    audit(
        conn,
        actor,
        "group.member.remove",
        "group",
        group_id,
        json!({"user_id": user_id}),
    )
    .await
}

fn distinct(ids: &[Uuid]) -> Vec<Uuid> {
    let mut ids = ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    ids
}

// ---------------------------------------------------------------------------------------------
// Subgroups
// ---------------------------------------------------------------------------------------------

/// Locks the group of the subgroup and returns the subgroup as it is once the lock is held. The
/// group id is read first without a lock: a subgroup never moves to another group.
async fn lock_subgroup(conn: &mut PgConnection, id: Uuid) -> Result<Subgroup, OrgError> {
    let first = repo::find_subgroup(conn, id)
        .await?
        .ok_or(OrgError::SubgroupNotFound)?;
    repo::lock_group(conn, first.group_id)
        .await?
        .ok_or(OrgError::SubgroupNotFound)?;
    repo::find_subgroup(conn, id)
        .await?
        .ok_or(OrgError::SubgroupNotFound)
}

fn subgroup_name_taken(error: sqlx::Error) -> OrgError {
    name_taken(error, "subgroups_name_key", OrgError::SubgroupNameTaken)
}

pub async fn create_subgroup(
    conn: &mut PgConnection,
    actor: &Actor,
    group_id: Uuid,
    name: &str,
) -> Result<Subgroup, OrgError> {
    policy::authorize(actor, Action::ManageGroups)?;
    let name = OrgName::parse(name)?;
    repo::lock_group(conn, group_id)
        .await?
        .ok_or(OrgError::GroupNotFound)?;
    let subgroup = repo::insert_subgroup(conn, Uuid::now_v7(), group_id, name.as_str())
        .await
        .map_err(subgroup_name_taken)?;
    audit(
        conn,
        actor,
        "subgroup.create",
        "subgroup",
        subgroup.id,
        json!({"group_id": group_id}),
    )
    .await?;
    Ok(subgroup)
}

pub async fn list_subgroups(
    conn: &mut PgConnection,
    actor: &Actor,
    group_id: Uuid,
    page: PageParams,
) -> Result<Page<Subgroup>, OrgError> {
    policy::authorize(actor, Action::ViewGroups)?;
    repo::find_group(conn, group_id)
        .await?
        .ok_or(OrgError::GroupNotFound)?;
    let rows = repo::list_subgroups(conn, group_id, page.after, i64::from(page.limit) + 1).await?;
    Ok(Page::from_rows(rows, page.limit, |subgroup| subgroup.id))
}

pub async fn rename_subgroup(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
    name: &str,
) -> Result<Subgroup, OrgError> {
    policy::authorize(actor, Action::ManageGroups)?;
    let name = OrgName::parse(name)?;
    lock_subgroup(conn, id).await?;
    let subgroup = repo::rename_subgroup(conn, id, name.as_str())
        .await
        .map_err(subgroup_name_taken)?;
    audit(conn, actor, "subgroup.rename", "subgroup", id, json!({})).await?;
    Ok(subgroup)
}

pub async fn delete_subgroup(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<(), OrgError> {
    policy::authorize(actor, Action::ManageGroups)?;
    lock_subgroup(conn, id).await?;
    repo::delete_subgroup(conn, id)
        .await
        .map_err(|e| in_use(e, OrgError::SubgroupInUse))?;
    audit(conn, actor, "subgroup.delete", "subgroup", id, json!({})).await
}

pub async fn list_subgroup_members(
    conn: &mut PgConnection,
    actor: &Actor,
    subgroup_id: Uuid,
    page: PageParams,
) -> Result<Page<GroupMember>, OrgError> {
    policy::authorize(actor, Action::ViewGroups)?;
    repo::find_subgroup(conn, subgroup_id)
        .await?
        .ok_or(OrgError::SubgroupNotFound)?;
    let rows =
        repo::list_subgroup_members(conn, subgroup_id, page.after, i64::from(page.limit) + 1)
            .await?;
    Ok(Page::from_rows(rows, page.limit, |member| member.user_id))
}

/// Adds the users to the subgroup and returns the ones who were not members yet. All or nothing:
/// every entry that is not a member of the group is reported with its position. The group
/// memberships are read once the group lock is held, so a removal we waited for is seen.
pub async fn add_subgroup_members(
    conn: &mut PgConnection,
    actor: &Actor,
    subgroup_id: Uuid,
    user_ids: &[Uuid],
) -> Result<Vec<Uuid>, OrgError> {
    policy::authorize(actor, Action::ManageGroups)?;
    let subgroup = lock_subgroup(conn, subgroup_id).await?;
    let in_group: HashSet<Uuid> = repo::group_members_among(conn, subgroup.group_id, user_ids)
        .await?
        .into_iter()
        .collect();
    let refused: Vec<(usize, MemberProblem)> = user_ids
        .iter()
        .enumerate()
        .filter(|(_, id)| !in_group.contains(id))
        .map(|(position, _)| (position, MemberProblem::NotInGroup))
        .collect();
    if !refused.is_empty() {
        return Err(OrgError::Members(refused));
    }
    let added =
        repo::insert_subgroup_members(conn, subgroup_id, subgroup.group_id, &distinct(user_ids))
            .await?;
    if !added.is_empty() {
        audit(
            conn,
            actor,
            "subgroup.member.add",
            "subgroup",
            subgroup_id,
            json!({"user_ids": added}),
        )
        .await?;
    }
    Ok(added)
}

pub async fn remove_subgroup_member(
    conn: &mut PgConnection,
    actor: &Actor,
    subgroup_id: Uuid,
    user_id: Uuid,
) -> Result<(), OrgError> {
    policy::authorize(actor, Action::ManageGroups)?;
    lock_subgroup(conn, subgroup_id).await?;
    if repo::delete_subgroup_member(conn, subgroup_id, user_id).await? == 0 {
        return Err(OrgError::MemberNotFound);
    }
    audit(
        conn,
        actor,
        "subgroup.member.remove",
        "subgroup",
        subgroup_id,
        json!({"user_id": user_id}),
    )
    .await
}

/// A group with these students in it, made the way an admin makes one. For the tests of other
/// modules that need a group; they must not touch `org`'s tables themselves.
#[cfg(test)]
pub async fn seed_group(
    conn: &mut PgConnection,
    admin: &Actor,
    name: &str,
    students: &[Uuid],
) -> Result<Uuid, OrgError> {
    let group = create_group(conn, admin, name).await?;
    add_group_members(conn, admin, group.id, students).await?;
    Ok(group.id)
}

// ---------------------------------------------------------------------------------------------
// The installation record
// ---------------------------------------------------------------------------------------------

/// Records the mode on the first start of the installation; on every later start checks that the
/// configuration still names it (ADR-0031). Acts for no user, so it takes no `Actor`. Runs on the
/// caller's transaction.
pub async fn record_mode(
    conn: &mut PgConnection,
    configured: InstitutionMode,
) -> Result<(), OrgError> {
    db::record_mode(conn, configured)
        .await
        .map_err(|e| match e {
            db::ModeError::Db(e) => OrgError::Db(e),
            db::ModeError::Unknown { recorded } => OrgError::Db(sqlx::Error::Protocol(format!(
                "org.installation holds the unknown mode \"{recorded}\""
            ))),
            db::ModeError::Changed {
                recorded,
                configured,
            } => OrgError::ModeChanged {
                recorded,
                configured,
            },
        })
}

#[cfg(test)]
mod tests;
