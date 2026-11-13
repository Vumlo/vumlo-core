//! Business rules. Every function that acts for a user authorizes first, takes the connection it
//! runs on and never begins or commits a transaction. Every write below a group takes the group
//! row lock first (ADR-0032).

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{
    DownloadUrl, Group, GroupMember, GroupOption, MemberLine, MemberProblem, ModelError, OrgName,
    Subgroup, Term, TermDates, TermUpdate, Unit, UnitCandidate, UnitRoleEntry, UnitUpdate,
};
use super::repo;
use access::Actor;
use access::{
    self, AccessError, Action, UnitAbility, UnitGrant, UnitRole, group_role_allowed,
    unit_role_allowed,
};
use auth::{self, AuthError};
use common::{CANDIDATE_LIMIT, CANDIDATE_MIN_CHARS, Page, PageParams, like_pattern};
use events::{self, AuditEntry, Event, EventsError};
use vumlo_config::InstitutionMode;

#[derive(Debug, thiserror::Error)]
pub enum OrgError {
    #[error(transparent)]
    Forbidden(#[from] AccessError),
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
    #[error("unit not found")]
    UnitNotFound,
    #[error("a unit with this name already exists here")]
    UnitNameTaken,
    #[error("a unit cannot move under itself or below")]
    UnitCycle,
    #[error("the unit has subunits or courses")]
    UnitInUse,
    #[error("this user cannot hold a unit role")]
    RoleNotAllowed,
    #[error("the search text is too short")]
    QueryTooShort,
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
    Identity(#[from] AuthError),
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

/// Tells `learning` that these users came or went in a group or one of its subgroups, so that it can
/// tell the plugins which rosters changed (ADR-0075). Nothing is sent for an empty list.
async fn members_changed(
    conn: &mut PgConnection,
    group_id: Uuid,
    user_ids: &[Uuid],
) -> Result<(), OrgError> {
    if !user_ids.is_empty() {
        events::emit(
            conn,
            Event::GroupMembersChanged {
                group_id,
                user_ids: user_ids.to_vec(),
            },
        )
        .await?;
    }
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
    access::authorize(actor, Action::ManageTerms)?;
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
    access::authorize(actor, Action::ViewTerms)?;
    let rows = repo::list_terms(conn, page.after, i64::from(page.limit) + 1).await?;
    Ok(Page::from_rows(rows, page.limit, |term| term.id))
}

/// Authorizes the admin view of the institution settings (ADR-0044). The settings themselves come
/// from the configuration the server parsed at startup, so there is nothing to read.
pub fn view_institution_settings(actor: &Actor) -> Result<(), OrgError> {
    access::authorize(actor, Action::ViewInstitutionSettings)?;
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
    access::authorize(actor, Action::ManageAppDownloads)?;
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
    access::authorize(actor, Action::ViewTerms)?;
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
    access::authorize(actor, Action::ManageTerms)?;
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
    access::authorize(actor, Action::ManageTerms)?;
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
    access::authorize(actor, Action::ManageGroups)?;
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
    access::authorize(actor, Action::ViewGroups)?;
    let rows = repo::list_groups(conn, page.after, i64::from(page.limit) + 1).await?;
    Ok(Page::from_rows(rows, page.limit, |group| group.id))
}

pub async fn get_group(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<Group, OrgError> {
    access::authorize(actor, Action::ViewGroups)?;
    repo::find_group(conn, id)
        .await?
        .ok_or(OrgError::GroupNotFound)
}

/// The people of a group with their names and emails, by surname (ADR-0053). Admins only.
pub async fn group_member_lines(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<Vec<MemberLine>, OrgError> {
    access::authorize(actor, Action::ViewGroups)?;
    repo::find_group(conn, id)
        .await?
        .ok_or(OrgError::GroupNotFound)?;
    Ok(repo::group_member_lines(conn, id).await?)
}

/// The groups a person is in, by name. The read interface `learning` uses for a person's facts;
/// the caller has authorized the administrator.
pub async fn groups_of_user(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<Vec<(Uuid, String)>, OrgError> {
    Ok(repo::groups_of_user(conn, user_id).await?)
}

/// Every term, newest first, for the administrator's term statistics (ADR-0053). Admins only.
pub async fn terms_newest_first(
    conn: &mut PgConnection,
    actor: &Actor,
) -> Result<Vec<Term>, OrgError> {
    access::authorize(actor, Action::ViewAdminReports)?;
    Ok(repo::all_terms_newest_first(conn).await?)
}

pub async fn rename_group(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
    name: &str,
) -> Result<Group, OrgError> {
    access::authorize(actor, Action::ManageGroups)?;
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
    access::authorize(actor, Action::ManageGroups)?;
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
/// authorized its own action, as with `auth::find_user`. Groups stay admin-only to read
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
    access::authorize(actor, Action::ViewGroups)?;
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
    let roles: HashMap<Uuid, common::Role> = auth::find_users(conn, &others)
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
    access::authorize(actor, Action::ManageGroups)?;
    repo::lock_group(conn, group_id)
        .await?
        .ok_or(OrgError::GroupNotFound)?;
    let members: HashSet<Uuid> = repo::group_members_among(conn, group_id, user_ids)
        .await?
        .into_iter()
        .collect();
    check_new_members(conn, user_ids, &members).await?;
    let added = repo::insert_group_members(conn, group_id, &distinct(user_ids)).await?;
    members_changed(conn, group_id, &added).await?;
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
    access::authorize(actor, Action::ManageGroups)?;
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
    members_changed(conn, group_id, &[added.clone(), removed.clone()].concat()).await?;
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
    access::authorize(actor, Action::ManageGroups)?;
    repo::lock_group(conn, group_id)
        .await?
        .ok_or(OrgError::GroupNotFound)?;
    let removed = repo::delete_group_members(conn, group_id, &distinct(user_ids)).await?;
    members_changed(conn, group_id, &removed).await?;
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
    access::authorize(actor, Action::ManageGroups)?;
    repo::lock_group(conn, group_id)
        .await?
        .ok_or(OrgError::GroupNotFound)?;
    if repo::delete_group_member(conn, group_id, user_id).await? == 0 {
        return Err(OrgError::MemberNotFound);
    }
    members_changed(conn, group_id, &[user_id]).await?;
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
    access::authorize(actor, Action::ManageGroups)?;
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
    access::authorize(actor, Action::ViewGroups)?;
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
    access::authorize(actor, Action::ManageGroups)?;
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
    access::authorize(actor, Action::ManageGroups)?;
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
    access::authorize(actor, Action::ViewGroups)?;
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
    access::authorize(actor, Action::ManageGroups)?;
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
    members_changed(conn, subgroup.group_id, &added).await?;
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
    access::authorize(actor, Action::ManageGroups)?;
    let subgroup = lock_subgroup(conn, subgroup_id).await?;
    if repo::delete_subgroup_member(conn, subgroup_id, user_id).await? == 0 {
        return Err(OrgError::MemberNotFound);
    }
    members_changed(conn, subgroup.group_id, &[user_id]).await?;
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

// ---------------------------------------------------------------------------------------------
// Units (ADR-0094)
// ---------------------------------------------------------------------------------------------

/// The grant stored abilities make on `unit_id`. A text that is no ability is a decode error
/// ([`UnitGrant::from_names`]).
fn grant_of(unit_id: Uuid, abilities: &[String]) -> Result<UnitGrant, OrgError> {
    UnitGrant::from_names(unit_id, abilities)
        .map_err(|message| sqlx::Error::Decode(message.into()).into())
}

/// The actor's grant on a unit that is known to exist, as `access` reads their roles there.
async fn grant_on(
    conn: &mut PgConnection,
    actor: &Actor,
    unit_id: Uuid,
) -> Result<Option<UnitGrant>, OrgError> {
    let held = match repo::unit_abilities(conn, actor.user_id(), unit_id).await? {
        Some(abilities) => Some(grant_of(unit_id, &abilities)?),
        None => None,
    };
    Ok(access::effective_unit_grant(actor, unit_id, held))
}

/// The grant on the parent of a unit; a top-level unit has none (only an admin manages it).
async fn parent_grant(
    conn: &mut PgConnection,
    actor: &Actor,
    parent_id: Option<Uuid>,
) -> Result<Option<UnitGrant>, OrgError> {
    match parent_id {
        Some(parent_id) => grant_on(conn, actor, parent_id).await,
        None => Ok(None),
    }
}

/// The actor's grant on a unit; `None` when they hold none. `UnitNotFound` if the unit does not exist.
pub async fn unit_grant(
    conn: &mut PgConnection,
    actor: &Actor,
    unit_id: Uuid,
) -> Result<Option<UnitGrant>, OrgError> {
    repo::find_unit(conn, unit_id)
        .await?
        .ok_or(OrgError::UnitNotFound)?;
    grant_on(conn, actor, unit_id).await
}

/// A unit the actor may see, with their grant on it. A unit they may not see does not exist for them.
async fn visible_unit(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
) -> Result<(Unit, Option<UnitGrant>), OrgError> {
    let unit = repo::find_unit(conn, id)
        .await?
        .ok_or(OrgError::UnitNotFound)?;
    let grant = grant_on(conn, actor, id).await?;
    access::authorize(actor, Action::ViewUnit(grant)).map_err(|_| OrgError::UnitNotFound)?;
    Ok((unit, grant))
}

/// The units the actor sees with their grant on each: an administrator all of them, everyone else
/// the ones their roles cover (none for most people).
pub async fn list_units(
    conn: &mut PgConnection,
    actor: &Actor,
) -> Result<Vec<(Unit, UnitGrant)>, OrgError> {
    access::authorize(actor, Action::ViewOwnUnits)?;
    let mut held = HashMap::new();
    for (unit, abilities) in repo::list_units_of_user(conn, actor.user_id()).await? {
        held.insert(unit.id, grant_of(unit.id, &abilities)?);
    }
    // ponytail: reads every unit for everyone, fine for hundreds; a query per role kind if it grows.
    let units = repo::list_units(conn).await?;
    Ok(units
        .into_iter()
        .filter_map(|unit| {
            let grant = access::effective_unit_grant(actor, unit.id, held.get(&unit.id).copied())?;
            Some((unit, grant))
        })
        .collect())
}

pub async fn create_unit(
    conn: &mut PgConnection,
    actor: &Actor,
    name: OrgName,
    parent_id: Option<Uuid>,
) -> Result<Unit, OrgError> {
    let grant = match parent_id {
        Some(parent_id) => visible_unit(conn, actor, parent_id).await?.1,
        None => None,
    };
    access::authorize(actor, Action::ManageUnits(grant))?;
    let unit = repo::insert_unit(conn, Uuid::now_v7(), parent_id, name.as_str())
        .await
        .map_err(|e| match &e {
            // The parent was deleted since we looked.
            sqlx::Error::Database(db) if db.is_foreign_key_violation() => OrgError::UnitNotFound,
            _ => name_taken(e, "units_name_key", OrgError::UnitNameTaken),
        })?;
    audit(
        conn,
        actor,
        "unit.create",
        "unit",
        unit.id,
        json!({"name": unit.name, "parent_id": unit.parent_id}),
    )
    .await?;
    Ok(unit)
}

/// What an update will do, once the actor is known to be allowed to do it.
struct UnitChange {
    unit: Unit,
    new_parent: Option<Uuid>,
    moved: bool,
    new_name: String,
    renamed: bool,
}

/// Reads the unit and checks every grant the update needs: the one on the unit's parent, and for a
/// move the new parent's visibility, its grant and the cycle rule. Run once without the tree lock,
/// so a caller who may not do it never takes the lock, and again under it, because the tree may have
/// changed in between.
async fn check_update(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
    update: &UnitUpdate,
) -> Result<UnitChange, OrgError> {
    let (unit, _) = visible_unit(conn, actor, id).await?;
    let old_grant = parent_grant(conn, actor, unit.parent_id).await?;
    access::authorize(actor, Action::ManageUnits(old_grant))?;

    let new_parent = update.parent_id.unwrap_or(unit.parent_id);
    let moved = new_parent != unit.parent_id;
    if moved {
        let new_grant = match new_parent {
            Some(new_parent) => {
                let (_, grant) = visible_unit(conn, actor, new_parent).await?;
                if repo::in_subtree(conn, id, new_parent).await? {
                    return Err(OrgError::UnitCycle);
                }
                grant
            }
            None => None,
        };
        access::authorize(actor, Action::ManageUnits(new_grant))?;
    }
    let new_name = update
        .name
        .as_ref()
        .map_or(unit.name.as_str(), OrgName::as_str)
        .to_owned();
    let renamed = new_name != unit.name;
    Ok(UnitChange {
        unit,
        new_parent,
        moved,
        new_name,
        renamed,
    })
}

/// Renames and/or moves a unit. The grant checked is the one on its parent, and for a move also the
/// one on the new parent. Everything is checked first without a lock. A move that passes takes the
/// tree lock, then reads and checks again (including that the new parent is not inside the unit)
/// before it writes. A rename alone writes the name and never the parent.
pub async fn update_unit(
    conn: &mut PgConnection,
    actor: &Actor,
    id: Uuid,
    update: UnitUpdate,
) -> Result<Unit, OrgError> {
    let mut change = check_update(conn, actor, id, &update).await?;
    if change.moved {
        repo::lock_tree(conn).await?;
        change = check_update(conn, actor, id, &update).await?;
    }
    let UnitChange {
        unit,
        new_parent,
        moved,
        new_name,
        renamed,
    } = change;
    if !moved && !renamed {
        return Ok(unit);
    }

    let written = if moved {
        // Under the lock; the name rides along so that the pair is checked as one.
        repo::move_unit(conn, id, new_parent, renamed.then_some(new_name.as_str()))
            .await
            .map_err(|e| name_taken(e, "units_name_key", OrgError::UnitNameTaken))?
    } else {
        repo::rename_unit(conn, id, &new_name)
            .await
            .map_err(|e| name_taken(e, "units_name_key", OrgError::UnitNameTaken))?
            .ok_or(OrgError::UnitNotFound)?
    };
    if renamed {
        audit(
            conn,
            actor,
            "unit.rename",
            "unit",
            id,
            json!({"name": written.name}),
        )
        .await?;
    }
    if moved {
        audit(
            conn,
            actor,
            "unit.move",
            "unit",
            id,
            json!({"name": written.name, "from": unit.parent_id, "to": new_parent}),
        )
        .await?;
    }
    Ok(written)
}

/// Deletes a unit with no subunits and no courses; the foreign keys refuse the rest.
pub async fn delete_unit(conn: &mut PgConnection, actor: &Actor, id: Uuid) -> Result<(), OrgError> {
    let (unit, _) = visible_unit(conn, actor, id).await?;
    let grant = parent_grant(conn, actor, unit.parent_id).await?;
    access::authorize(actor, Action::ManageUnits(grant))?;
    let deleted = repo::delete_unit(conn, id).await.map_err(|e| match &e {
        // `units.parent_id` and `courses.unit_id` have no cascade: NO ACTION reports 23503.
        sqlx::Error::Database(db) if db.is_foreign_key_violation() => OrgError::UnitInUse,
        _ => OrgError::Db(e),
    })?;
    if deleted == 0 {
        return Err(OrgError::UnitNotFound);
    }
    audit(
        conn,
        actor,
        "unit.delete",
        "unit",
        id,
        json!({"name": unit.name, "parent_id": unit.parent_id}),
    )
    .await
}

// ---------------------------------------------------------------------------------------------
// Unit roles and the abilities admins switch (ADR-0094)
// ---------------------------------------------------------------------------------------------

/// The unit, once the actor is known to see it and to hold `manage_units` on its parent (an
/// administrator for a top-level unit): the check behind granting, revoking and searching for
/// people. Takes no lock, so a caller who is refused has not delayed anyone.
async fn unit_managed_from_above(
    conn: &mut PgConnection,
    actor: &Actor,
    unit_id: Uuid,
) -> Result<Unit, OrgError> {
    let (unit, _) = visible_unit(conn, actor, unit_id).await?;
    let grant = parent_grant(conn, actor, unit.parent_id).await?;
    access::authorize(actor, Action::ManageUnits(grant))?;
    Ok(unit)
}

/// The roles held on this unit itself, by surname. Anyone who sees the unit may list them.
pub async fn list_unit_roles(
    conn: &mut PgConnection,
    actor: &Actor,
    unit_id: Uuid,
) -> Result<Vec<UnitRoleEntry>, OrgError> {
    visible_unit(conn, actor, unit_id).await?;
    Ok(repo::list_unit_roles(conn, unit_id).await?)
}

/// Gives a teacher or an administrator a role on the unit (replacing the one they held). Needs
/// `manage_units` on the unit's parent, so nobody changes the roles of the unit their own role sits
/// on. The person's row is read `FOR SHARE` after the caller is authorized, so a demotion cannot slip
/// in between the check of the institution role and the write (ADR-0037), and a refused caller never
/// waits for that lock.
pub async fn set_unit_role(
    conn: &mut PgConnection,
    actor: &Actor,
    unit_id: Uuid,
    user_id: Uuid,
    role: UnitRole,
) -> Result<(), OrgError> {
    unit_managed_from_above(conn, actor, unit_id).await?;
    let user = auth::find_user_shared(conn, user_id)
        .await?
        .ok_or(OrgError::MemberNotFound)?;
    // The lock holds the person's institution role. The caller's grant is not decided by the
    // person's row: it comes from the tree and the caller's own roles, and waiting on the lock
    // changes neither, so it is not checked again.
    if !unit_role_allowed(user.role) {
        return Err(OrgError::RoleNotAllowed);
    }
    let changed = repo::upsert_unit_role(conn, unit_id, user_id, role)
        .await
        .map_err(|e| match &e {
            // The unit was deleted since we looked.
            sqlx::Error::Database(db) if db.is_foreign_key_violation() => OrgError::UnitNotFound,
            _ => OrgError::Db(e),
        })?;
    if changed {
        audit(
            conn,
            actor,
            "unit.role.set",
            "unit",
            unit_id,
            json!({"user_id": user_id, "role": role.as_str()}),
        )
        .await?;
    }
    Ok(())
}

/// Takes a person's role on the unit away. Needs `manage_units` on the unit's parent.
pub async fn remove_unit_role(
    conn: &mut PgConnection,
    actor: &Actor,
    unit_id: Uuid,
    user_id: Uuid,
) -> Result<(), OrgError> {
    unit_managed_from_above(conn, actor, unit_id).await?;
    let role = repo::delete_unit_role(conn, unit_id, user_id)
        .await?
        .ok_or(OrgError::MemberNotFound)?;
    audit(
        conn,
        actor,
        "unit.role.remove",
        "unit",
        unit_id,
        json!({"user_id": user_id, "role": role.as_str()}),
    )
    .await
}

/// Takes every unit role of a person who stopped being a teacher, and audits each. For
/// `learning::people::update_user`, which has authorized `ManageUsers` and holds the person's row
/// (the way it calls `teaching::drop_teacher_memberships`).
pub async fn drop_unit_roles(
    conn: &mut PgConnection,
    actor: &Actor,
    user_id: Uuid,
) -> Result<(), OrgError> {
    for (unit_id, role) in repo::delete_roles_of_user(conn, user_id).await? {
        audit(
            conn,
            actor,
            "unit.role.remove",
            "unit",
            unit_id,
            json!({"user_id": user_id, "role": role.as_str(), "reason": "role_change"}),
        )
        .await?;
    }
    Ok(())
}

/// Active teachers and administrators without a role on the unit whose name or email contains `q`.
/// Needs `manage_units` on the unit's parent, like granting does.
pub async fn unit_candidates(
    conn: &mut PgConnection,
    actor: &Actor,
    unit_id: Uuid,
    q: &str,
) -> Result<Vec<UnitCandidate>, OrgError> {
    unit_managed_from_above(conn, actor, unit_id).await?;
    let q = q.trim();
    if q.chars().count() < CANDIDATE_MIN_CHARS {
        return Err(OrgError::QueryTooShort);
    }
    Ok(repo::search_unit_candidates(conn, unit_id, &like_pattern(q), CANDIDATE_LIMIT).await?)
}

/// The switches each unit role has now: every role, with its abilities in the order of
/// [`UnitAbility::ALL`]. Administrators only.
pub async fn unit_role_abilities(
    conn: &mut PgConnection,
    actor: &Actor,
) -> Result<Vec<(UnitRole, Vec<UnitAbility>)>, OrgError> {
    access::authorize(actor, Action::ManageUnitRoleAbilities)?;
    let on = repo::list_role_abilities(conn).await?;
    Ok([UnitRole::Manager, UnitRole::Viewer]
        .into_iter()
        .map(|role| {
            let abilities = UnitAbility::ALL
                .into_iter()
                .filter(|ability| on.contains(&(role, *ability)))
                .collect();
            (role, abilities)
        })
        .collect())
}

/// Replaces the switches of a unit role by exactly these, in one statement pair under a table lock.
/// Every request after it reads the new set (nothing caches grants). Administrators only.
pub async fn set_unit_role_abilities(
    conn: &mut PgConnection,
    actor: &Actor,
    role: UnitRole,
    abilities: Vec<UnitAbility>,
) -> Result<(), OrgError> {
    access::authorize(actor, Action::ManageUnitRoleAbilities)?;
    // Sorted and without duplicates, whatever order they came in.
    let abilities: Vec<UnitAbility> = UnitAbility::ALL
        .into_iter()
        .filter(|ability| abilities.contains(ability))
        .collect();
    let names: Vec<&str> = abilities.iter().map(|ability| ability.as_str()).collect();
    repo::lock_role_abilities(conn).await?;
    repo::replace_role_abilities(conn, role, &names).await?;
    audit(
        conn,
        actor,
        "unit.abilities.set",
        "unit_role",
        // A role has no row of its own: the entry names the installation by the nil id.
        Uuid::nil(),
        json!({"role": role.as_str(), "abilities": names}),
    )
    .await
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
