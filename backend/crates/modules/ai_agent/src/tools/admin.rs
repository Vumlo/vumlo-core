//! The tools for people and the organisation, for an administrator (ADR-0070): two lookups and four
//! changes. Like every pack, each change is a call of the core's own service function as the user,
//! so authorization, validation and the audit row are the core's.
//!
//! A password reset is the one change that carries a secret. Its temporary password is made and
//! hashed by the confirm route, not here, and goes only into the response of that request
//! (`no-store`): it is in no proposal, no chat message and no model prompt.

use std::collections::BTreeMap;

use access::{Action, Actor};
use auth::{HashedPassword, User, UserFilter};
use chrono::NaiveDate;
use common::{ApiError, PageParams};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use super::{args, capped, explain, unavailable};
use crate::llm::ToolSpec;

/// Rows asked of the core at a time, and the most pages one lookup reads.
const PAGE: u32 = 200;
const MAX_PAGES: usize = 5;
/// People one change may add to a group.
const MAX_PEOPLE: usize = 50;

/// A change to people or the organisation, as the model asked for it and the user will see it. It
/// is stored as JSON in the proposal and parsed again at confirm.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case", deny_unknown_fields)]
pub enum Write {
    CreateTerm {
        name: String,
        starts_on: NaiveDate,
        ends_on: NaiveDate,
    },
    CreateGroup {
        name: String,
    },
    AddGroupMembers {
        group_id: Uuid,
        user_ids: Vec<Uuid>,
    },
    ResetPassword {
        user_id: Uuid,
    },
}

pub const NAMES: [&str; 4] = [
    "create_term",
    "create_group",
    "add_group_members",
    "reset_password",
];

pub fn specs() -> Vec<ToolSpec> {
    let text = |what: &str| json!({"type": "string", "description": what});
    vec![
        ToolSpec::new(
            "find_people",
            "Looks people up by part of their name, surname or email. Gives their ids, emails, roles \
             and whether the account is active. Use it to find the ids a change needs.",
            json!({
                "type": "object",
                "properties": {
                    "query": text("Part of a name, surname or email"),
                    "role": {"type": ["string", "null"], "description": "Only this role: student, teacher or admin"}
                },
                "required": ["query"]
            }),
        ),
        ToolSpec::new(
            "org_overview",
            "The terms of the institution with their dates, and its groups (classes) with their ids. \
             Use it to find the ids a change needs.",
            json!({
                "type": "object",
                "properties": {"query": {"type": ["string", "null"], "description": "Only the groups whose name contains this"}}
            }),
        ),
        ToolSpec::new(
            "create_term",
            "Prepares creating a term. The user must confirm before anything is created.",
            json!({
                "type": "object",
                "properties": {
                    "name": text("The name of the term, such as Autumn 2026"),
                    "starts_on": text("The first day, as YYYY-MM-DD"),
                    "ends_on": text("The last day, as YYYY-MM-DD")
                },
                "required": ["name", "starts_on", "ends_on"]
            }),
        ),
        ToolSpec::new(
            "create_group",
            "Prepares creating an empty group (a class). The user must confirm before anything is created.",
            json!({
                "type": "object",
                "properties": {"name": text("The name of the group, such as 7B")},
                "required": ["name"]
            }),
        ),
        ToolSpec::new(
            "add_group_members",
            "Prepares adding students to a group. Only students can be members. Up to 50 at a time; \
             all or none are added. The user must confirm before anything is added.",
            json!({
                "type": "object",
                "properties": {
                    "group_id": text("A group id from org_overview"),
                    "user_ids": {"type": "array", "items": {"type": "string"}, "description": "Student ids from find_people"}
                },
                "required": ["group_id", "user_ids"]
            }),
        ),
        ToolSpec::new(
            "reset_password",
            "Prepares replacing one person's password with a temporary one, which signs them out \
             everywhere. The temporary password is shown to the user only when they confirm; you \
             will never see it. The user must confirm before anything changes.",
            json!({
                "type": "object",
                "properties": {"user_id": text("A person's id from find_people")},
                "required": ["user_id"]
            }),
        ),
    ]
}

impl Write {
    /// Reads what the model sent. Everything is untrusted.
    pub fn parse(name: &str, arguments: &str) -> Result<Self, String> {
        let raw = if arguments.trim().is_empty() {
            "{}"
        } else {
            arguments
        };
        let mut value: Value = serde_json::from_str(raw)
            .map_err(|_| "the arguments do not fit this tool".to_owned())?;
        let object = value
            .as_object_mut()
            .ok_or("the arguments do not fit this tool")?;
        object.insert("tool".to_owned(), Value::String(name.to_owned()));
        let write: Self = serde_json::from_value(value)
            .map_err(|e| format!("the arguments do not fit {name}: {e}"))?;
        if let Self::AddGroupMembers { user_ids, .. } = &write
            && !(1..=MAX_PEOPLE).contains(&user_ids.len())
        {
            return Err(format!("send between 1 and {MAX_PEOPLE} people at a time"));
        }
        Ok(write)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::CreateTerm { .. } => NAMES[0],
            Self::CreateGroup { .. } => NAMES[1],
            Self::AddGroupMembers { .. } => NAMES[2],
            Self::ResetPassword { .. } => NAMES[3],
        }
    }

    /// Whether the confirm needs a temporary password made for it.
    pub fn wants_password(&self) -> bool {
        matches!(self, Self::ResetPassword { .. })
    }

    /// What the card will say, read from the service as the user. A lookup of someone else's data
    /// is authorized here first: the user lookups of `auth` check nothing themselves.
    pub async fn describe(
        &self,
        conn: &mut PgConnection,
        actor: &Actor,
    ) -> Result<BTreeMap<&'static str, String>, String> {
        let mut facts = BTreeMap::new();
        match self {
            Self::CreateTerm {
                name,
                starts_on,
                ends_on,
            } => {
                facts.insert("name", name.trim().to_owned());
                facts.insert("starts_on", starts_on.to_string());
                facts.insert("ends_on", ends_on.to_string());
            }
            Self::CreateGroup { name } => {
                facts.insert("name", name.trim().to_owned());
            }
            Self::AddGroupMembers { group_id, user_ids } => {
                allow(actor, Action::ManageGroups)?;
                allow(actor, Action::ManageUsers)?;
                let group = org::get_group(conn, actor, *group_id)
                    .await
                    .map_err(|e| explain(&ApiError::from(e)))?;
                let mut distinct = user_ids.clone();
                distinct.sort();
                distinct.dedup();
                let users = auth::find_users(conn, &distinct)
                    .await
                    .map_err(|e| explain(&ApiError::from(e)))?;
                if users.len() != distinct.len() {
                    return Err("some of those people do not exist".to_owned());
                }
                facts.insert("group", group.name);
                facts.insert("count", users.len().to_string());
                facts.insert(
                    "people",
                    users.iter().map(person).collect::<Vec<_>>().join("; "),
                );
            }
            Self::ResetPassword { user_id } => {
                allow(actor, Action::ManageUsers)?;
                if *user_id == actor.user_id() {
                    return Err(
                        "that is the user's own account: they change their password on their account page"
                            .to_owned(),
                    );
                }
                let user = auth::find_user(conn, *user_id)
                    .await
                    .map_err(|e| explain(&ApiError::from(e)))?
                    .ok_or("no such person")?;
                facts.insert("person", format!("{} {}", user.name, user.surname));
                facts.insert("email", user.email);
                facts.insert("role", user.role.as_str().to_owned());
            }
        }
        Ok(facts)
    }

    /// Makes the change. The caller owns the transaction. `password` is the hash of the temporary
    /// password for a reset, made before the transaction began.
    pub async fn run(
        &self,
        conn: &mut PgConnection,
        actor: &Actor,
        password: Option<&HashedPassword>,
    ) -> Result<(), ApiError> {
        match self {
            Self::CreateTerm {
                name,
                starts_on,
                ends_on,
            } => {
                org::create_term(conn, actor, name, *starts_on, *ends_on).await?;
            }
            Self::CreateGroup { name } => {
                org::create_group(conn, actor, name).await?;
            }
            Self::AddGroupMembers { group_id, user_ids } => {
                org::add_group_members(conn, actor, *group_id, user_ids).await?;
            }
            Self::ResetPassword { user_id } => {
                let Some(password) = password else {
                    tracing::error!("ai_agent: a password reset ran without a password");
                    return Err(ApiError::Internal);
                };
                auth::reset_password(conn, actor, *user_id, password.clone()).await?;
            }
        }
        Ok(())
    }
}

fn allow(actor: &Actor, action: Action) -> Result<(), String> {
    access::authorize(actor, action).map_err(|e| explain(&ApiError::from(e)))
}

fn person(user: &User) -> String {
    format!("{} {} ({})", user.name, user.surname, user.email)
}

#[derive(Deserialize)]
struct FindPeople {
    query: String,
    role: Option<String>,
}

/// People whose email, name or surname contains the query. The email is searched by the core;
/// names are searched in the first few pages only.
// ponytail: names are matched in memory over MAX_PAGES * PAGE people; a core filter on names would
// replace this scan if an institution outgrows it.
pub async fn find_people(
    conn: &mut PgConnection,
    actor: &Actor,
    arguments: &str,
) -> Result<Value, String> {
    let FindPeople { query, role } = args(arguments)?;
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Err("query is required".to_owned());
    }
    let role = role
        .map(|r| r.trim().to_lowercase())
        .filter(|r| !r.is_empty());
    let wanted = |user: &User| role.as_deref().is_none_or(|r| user.role.as_str() == r);

    let mut found: Vec<User> = Vec::new();
    let mut after = None;
    for _ in 0..MAX_PAGES {
        let filter = UserFilter {
            email_contains: Some(needle.clone()),
            ..UserFilter::default()
        };
        let page = auth::list_users(conn, actor, filter, PageParams { after, limit: PAGE })
            .await
            .map_err(|e| unavailable("find_people", e))?;
        found.extend(page.items.into_iter().filter(&wanted));
        match page.next {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    if found.is_empty() {
        let mut after = None;
        for _ in 0..MAX_PAGES {
            let page = auth::list_users(
                conn,
                actor,
                UserFilter::default(),
                PageParams { after, limit: PAGE },
            )
            .await
            .map_err(|e| unavailable("find_people", e))?;
            found.extend(page.items.into_iter().filter(|u| {
                wanted(u)
                    && format!("{} {}", u.name, u.surname)
                        .to_lowercase()
                        .contains(&needle)
            }));
            match page.next {
                Some(next) => after = Some(next),
                None => break,
            }
        }
    }
    Ok(json!({
        "people": capped(&found, |u| json!({
            "id": u.id,
            "name": u.name,
            "surname": u.surname,
            "email": u.email,
            "role": u.role.as_str(),
            "status": u.status.as_str(),
        })),
    }))
}

#[derive(Deserialize)]
struct OrgQuery {
    query: Option<String>,
}

pub async fn org_overview(
    conn: &mut PgConnection,
    actor: &Actor,
    arguments: &str,
) -> Result<Value, String> {
    let OrgQuery { query } = args(arguments)?;
    let needle = query
        .map(|q| q.trim().to_lowercase())
        .filter(|q| !q.is_empty());
    let terms = org::list_terms(
        conn,
        actor,
        PageParams {
            after: None,
            limit: PAGE,
        },
    )
    .await
    .map_err(|e| unavailable("org_overview", e))?;

    let mut groups = Vec::new();
    let mut after = None;
    for _ in 0..MAX_PAGES {
        let page = org::list_groups(conn, actor, PageParams { after, limit: PAGE })
            .await
            .map_err(|e| unavailable("org_overview", e))?;
        groups.extend(page.items.into_iter().filter(|g| {
            needle
                .as_deref()
                .is_none_or(|n| g.name.to_lowercase().contains(n))
        }));
        match page.next {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    Ok(json!({
        "terms": capped(&terms.items, |t| json!({
            "id": t.id,
            "name": t.name,
            "starts_on": t.starts_on.to_string(),
            "ends_on": t.ends_on.to_string(),
        })),
        "groups": capped(&groups, |g| json!({"id": g.id, "name": g.name})),
    }))
}

#[cfg(test)]
mod tests;
