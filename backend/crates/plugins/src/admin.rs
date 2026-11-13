//! What an administrator does to plugins from the console (ADR-0095): the actor-checked and audited
//! side of the functions `vumlo-cli plugin ...` calls. Each function authorizes first, then calls the
//! actor-free function of `service` and writes the audit entry in the same transaction. Nothing here
//! installs or removes a plugin (that starts and stops processes on the host), and no token or
//! secret value passes through.

use access::{Action, Actor, CourseScope, Scope};
use events::AuditEntry;
use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::error::PluginsError;
use crate::model::{Install, InstallStatus, InstallSummary};
use crate::secret::SecretKey;
use crate::service::{self, CallReport, SettingView};

/// An install with the keys of its institution settings that still need a value.
pub struct AdminInstall {
    pub summary: InstallSummary,
    pub needs_setup: Vec<String>,
}

async fn needs_setup(
    conn: &mut PgConnection,
    plugin_id: &str,
) -> Result<Vec<String>, PluginsError> {
    Ok(service::list_settings(conn, plugin_id, None)
        .await?
        .into_iter()
        .filter(|view| !view.is_set && view.spec.default.is_none())
        .map(|view| view.key)
        .collect())
}

/// The audit entry of a change to one install. `meta` holds the plugin id and names, never a value.
fn entry(action: &'static str, install: &Install, meta: Value) -> AuditEntry {
    AuditEntry {
        action,
        resource_type: "plugin",
        resource_id: install.id,
        meta,
    }
}

fn scope_names(scopes: &[Scope]) -> Vec<&'static str> {
    scopes.iter().map(|scope| scope.as_str()).collect()
}

fn courses_json(courses: &CourseScope) -> Value {
    match courses {
        CourseScope::All => json!("all"),
        CourseScope::Only(list) => json!(list),
    }
}

/// Every install, with its deliveries and what still needs setting.
pub async fn admin_list(
    conn: &mut PgConnection,
    actor: &Actor,
) -> Result<Vec<AdminInstall>, PluginsError> {
    access::authorize(actor, Action::ViewPlugins)?;
    let mut found = Vec::new();
    for summary in service::list(conn).await? {
        let needs_setup = needs_setup(conn, &summary.install.plugin_id).await?;
        found.push(AdminInstall {
            summary,
            needs_setup,
        });
    }
    Ok(found)
}

pub async fn admin_get(
    conn: &mut PgConnection,
    actor: &Actor,
    plugin_id: &str,
) -> Result<AdminInstall, PluginsError> {
    access::authorize(actor, Action::ViewPlugins)?;
    let install = service::get(conn, plugin_id).await?;
    let needs_setup = needs_setup(conn, plugin_id).await?;
    // The deliveries come with the list; one install is found there.
    let summary = service::list(conn)
        .await?
        .into_iter()
        .find(|summary| summary.install.id == install.id)
        .ok_or_else(|| PluginsError::NotFound(plugin_id.to_owned()))?;
    Ok(AdminInstall {
        summary,
        needs_setup,
    })
}

/// The institution settings of a plugin. A secret shows only whether it is set.
pub async fn admin_settings(
    conn: &mut PgConnection,
    actor: &Actor,
    plugin_id: &str,
) -> Result<Vec<SettingView>, PluginsError> {
    access::authorize(actor, Action::ViewPlugins)?;
    service::list_settings(conn, plugin_id, None).await
}

/// Switches a plugin on or off. Off, its queued mail is cancelled in the same transaction. Nothing
/// changes, and nothing is audited, when it already is in that state.
pub async fn admin_set_status(
    conn: &mut PgConnection,
    actor: &Actor,
    plugin_id: &str,
    status: InstallStatus,
) -> Result<Install, PluginsError> {
    access::authorize(actor, Action::ManagePlugins)?;
    let before = service::get(conn, plugin_id).await?;
    if before.status == status {
        return Ok(before);
    }
    let after = service::set_status(conn, plugin_id, status).await?;
    let action = match status {
        InstallStatus::Disabled => "plugin.disable",
        InstallStatus::Enabled => "plugin.enable",
    };
    events::audit(
        conn,
        actor,
        entry(action, &after, json!({"plugin_id": after.plugin_id})),
    )
    .await?;
    Ok(after)
}

/// Replaces the scopes granted and the courses reached; the scopes stay within what the plugin asked
/// for.
pub async fn admin_grant(
    conn: &mut PgConnection,
    actor: &Actor,
    plugin_id: &str,
    scopes: &[Scope],
    courses: &CourseScope,
) -> Result<Install, PluginsError> {
    access::authorize(actor, Action::ManagePlugins)?;
    let before = service::get(conn, plugin_id).await?;
    let after = service::grant(conn, plugin_id, scopes, courses).await?;
    events::audit(
        conn,
        actor,
        entry(
            "plugin.grant",
            &after,
            json!({
                "plugin_id": after.plugin_id,
                "from": scope_names(&before.scopes),
                "to": scope_names(&after.scopes),
                "courses_from": courses_json(&before.courses),
                "courses_to": courses_json(&after.courses),
            }),
        ),
    )
    .await?;
    Ok(after)
}

/// Stores a setting typed as text; the setting's own type reads it. Returns whether anything
/// changed: the value it has already is left alone, announces nothing and is not audited.
pub async fn admin_set_setting(
    conn: &mut PgConnection,
    actor: &Actor,
    key: &SecretKey,
    plugin_id: &str,
    name: &str,
    text: &str,
) -> Result<bool, PluginsError> {
    access::authorize(actor, Action::ManagePlugins)?;
    let install = service::get(conn, plugin_id).await?;
    let changed = service::set_setting_text(conn, key, plugin_id, name, text, None).await?;
    if changed {
        // Never the value: it may be a credential.
        events::audit(
            conn,
            actor,
            entry(
                "plugin.setting.set",
                &install,
                json!({"plugin_id": install.plugin_id, "key": name, "course_id": null}),
            ),
        )
        .await?;
    }
    Ok(changed)
}

/// Removes a value, so that the default applies again. Returns whether there was one.
pub async fn admin_unset_setting(
    conn: &mut PgConnection,
    actor: &Actor,
    plugin_id: &str,
    name: &str,
) -> Result<bool, PluginsError> {
    access::authorize(actor, Action::ManagePlugins)?;
    let install = service::get(conn, plugin_id).await?;
    let removed = service::unset_setting(conn, plugin_id, name, None).await?;
    if removed {
        events::audit(
            conn,
            actor,
            entry(
                "plugin.setting.unset",
                &install,
                json!({"plugin_id": install.plugin_id, "key": name, "course_id": null}),
            ),
        )
        .await?;
    }
    Ok(removed)
}

pub async fn admin_calls(
    conn: &mut PgConnection,
    actor: &Actor,
    plugin_id: &str,
    since: chrono::DateTime<chrono::Utc>,
    limit: i64,
) -> Result<CallReport, PluginsError> {
    access::authorize(actor, Action::ViewPlugins)?;
    service::calls(conn, plugin_id, since, limit).await
}
