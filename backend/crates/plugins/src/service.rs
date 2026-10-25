//! Business rules. A function that acts for a plugin authorizes its scope first; the install
//! functions are for the administrator CLI, which has no `Actor` and audits what it changes
//! (source `cli`). Nothing here begins or commits a transaction.

use std::time::Duration;

use access::{self, Actor};
use access::{PluginCaller, Scope};
use events::OutboxEvent;
use learning::assessment;
use learning::content::{self, OpenError};
use module_api::OutboundClient;
use serde_json::{Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::PluginsError;
use crate::manifest::parse_manifest;
use crate::model::{
    ASSIGNMENT_TAB, Install, InstallStatus, InstallSummary, Installed, PluginManifest, PluginToken,
};
use crate::repo::{self, NewInstall, ReportRow, scope_names};
use crate::secret::{LaunchClaims, SecretKey, derive_secret, launch_token};

const MANIFEST_CONNECT: Duration = Duration::from_secs(3);
const MANIFEST_TOTAL: Duration = Duration::from_secs(5);
const MANIFEST_MAX_BYTES: usize = 64 * 1024;

/// A base URL the core will call: `https://`, or `http://` for a service on this machine. No
/// credentials, query or fragment (`vumlo_config::check_endpoint`). A trailing `/` is dropped.
pub fn check_base_url(url: &str) -> Result<String, PluginsError> {
    vumlo_config::check_endpoint(url, true).map_err(PluginsError::BaseUrl)?;
    Ok(url.trim_end_matches('/').to_owned())
}

/// `GET {base_url}/manifest`: five seconds, 64 KiB, no redirect (a plugin that moves elsewhere is
/// installed from where it is). Does not touch the database, so call it before opening a
/// transaction.
pub async fn fetch_manifest(base_url: &str) -> Result<PluginManifest, PluginsError> {
    let base = check_base_url(base_url)?;
    let client = OutboundClient::new(MANIFEST_CONNECT, MANIFEST_TOTAL, MANIFEST_MAX_BYTES, 0)
        .map_err(|e| PluginsError::Fetch(e.to_string()))?;
    let (status, body) = client
        .get(&format!("{base}/manifest"))
        .await
        .map_err(|e| PluginsError::Fetch(e.to_string()))?;
    if status != 200 {
        return Err(PluginsError::Fetch(format!(
            "the plugin answered HTTP {status}"
        )));
    }
    parse_manifest(&body)
}

fn unique(scopes: &[Scope]) -> Vec<Scope> {
    let mut found: Vec<Scope> = Vec::new();
    for scope in scopes {
        if !found.contains(scope) {
            found.push(*scope);
        }
    }
    found.sort();
    found
}

/// Registers the plugin with the scopes the administrator granted, which are at most the ones it
/// asked for. Returns the token and the secret: this is the only time either is shown.
pub async fn install(
    conn: &mut PgConnection,
    key: &SecretKey,
    base_url: &str,
    manifest: &PluginManifest,
    granted: &[Scope],
) -> Result<Installed, PluginsError> {
    let base_url = check_base_url(base_url)?;
    if let Some(extra) = granted.iter().find(|s| !manifest.scopes.contains(s)) {
        return Err(PluginsError::NotRequested(extra.as_str().to_owned()));
    }
    let granted = unique(granted);
    let id = Uuid::now_v7();
    let token = PluginToken::generate();
    let ui: Vec<Value> = manifest
        .ui
        .iter()
        .map(|s| json!({"slot": s.slot, "title": s.title, "path": s.path}))
        .collect();
    let inserted = repo::insert_install(
        conn,
        NewInstall {
            id,
            plugin_id: &manifest.id,
            version: &manifest.version,
            base_url: &base_url,
            subscribes: &manifest.subscribes,
            requested_scopes: &scope_names(&manifest.scopes),
            scopes: &scope_names(&granted),
            ui_slots: Value::Array(ui),
            token_hash: &token.hash(),
        },
    )
    .await;
    if let Err(error) = inserted {
        return Err(
            match error.as_database_error().and_then(|e| e.code()).as_deref() {
                Some("23505") => PluginsError::AlreadyInstalled(manifest.id.clone()),
                _ => PluginsError::Db(error),
            },
        );
    }
    let install = repo::find_install(conn, &manifest.id)
        .await?
        .ok_or_else(|| PluginsError::NotFound(manifest.id.clone()))?;
    let secret = derive_secret(key, install.id, install.secret_version)?;
    Ok(Installed {
        install,
        token,
        secret,
    })
}

pub async fn get(conn: &mut PgConnection, plugin_id: &str) -> Result<Install, PluginsError> {
    repo::find_install(conn, plugin_id)
        .await?
        .ok_or_else(|| PluginsError::NotFound(plugin_id.to_owned()))
}

/// Every install with the state of its deliveries.
pub async fn list(conn: &mut PgConnection) -> Result<Vec<InstallSummary>, PluginsError> {
    let counts = repo::delivery_counts(conn).await?;
    Ok(repo::list_installs(conn)
        .await?
        .into_iter()
        .map(|install| {
            let (pending, failed) = counts
                .iter()
                .find(|(id, _, _)| *id == install.id)
                .map_or((0, 0), |(_, pending, failed)| (*pending, *failed));
            InstallSummary {
                install,
                pending,
                failed,
            }
        })
        .collect())
}

/// Replaces the granted scopes. They stay within what the plugin asked for.
pub async fn grant(
    conn: &mut PgConnection,
    plugin_id: &str,
    scopes: &[Scope],
) -> Result<Install, PluginsError> {
    let install = get(conn, plugin_id).await?;
    if let Some(extra) = scopes
        .iter()
        .find(|s| !install.requested_scopes.contains(s))
    {
        return Err(PluginsError::NotRequested(extra.as_str().to_owned()));
    }
    repo::set_scopes(conn, install.id, &scope_names(&unique(scopes))).await?;
    get(conn, plugin_id).await
}

/// A disabled plugin's token stops working at once, it is sent no event, and the deliveries it was
/// owed wait until it is enabled again.
pub async fn set_status(
    conn: &mut PgConnection,
    plugin_id: &str,
    status: InstallStatus,
) -> Result<Install, PluginsError> {
    let install = get(conn, plugin_id).await?;
    repo::set_status(conn, install.id, status).await?;
    get(conn, plugin_id).await
}

/// Deletes the install, its pending deliveries and its reports. Returns what was removed.
pub async fn remove(conn: &mut PgConnection, plugin_id: &str) -> Result<Install, PluginsError> {
    let install = get(conn, plugin_id).await?;
    repo::delete_install(conn, install.id).await?;
    Ok(install)
}

/// Owes every enabled plugin that subscribed to the event a delivery. Returns how many it
/// created; a second run for the same event creates none.
pub async fn fan_out(conn: &mut PgConnection, event: &OutboxEvent) -> Result<u64, PluginsError> {
    let mut owed = 0;
    for install_id in repo::enabled_subscribers(conn, &event.kind).await? {
        if repo::insert_delivery(conn, install_id, event.id, &event.kind, &event.payload).await? {
            owed += 1;
        }
    }
    Ok(owed)
}

/// A page that an enabled plugin offers in a slot, with its full address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotPage {
    pub plugin_id: String,
    pub title: String,
    pub url: String,
}

/// The pages the enabled plugins offer in `slot`, by plugin. The address is the plugin's base URL
/// and the path it declared; the web app frames nothing else (ADR-0066).
pub async fn slots(conn: &mut PgConnection, slot: &str) -> Result<Vec<SlotPage>, PluginsError> {
    Ok(repo::list_installs(conn)
        .await?
        .into_iter()
        .filter(|install| install.status == InstallStatus::Enabled)
        .flat_map(|install| {
            let base = install.base_url;
            let plugin_id = install.plugin_id;
            install
                .ui
                .into_iter()
                .filter(|page| page.slot == slot)
                .map(move |page| SlotPage {
                    plugin_id: plugin_id.clone(),
                    title: page.title,
                    url: format!("{base}{}", page.path),
                })
        })
        .collect())
}

/// A token for the page of `plugin_id` in the tab of `assignment_id`, for someone who can see that
/// assignment. Everything that is not found is the same `NotFound`: the assignment (or a draft, or
/// one in a course the caller cannot see), an unknown or disabled plugin, and one that offers no
/// tab.
pub async fn launch(
    conn: &mut PgConnection,
    actor: &Actor,
    key: &SecretKey,
    assignment_id: Uuid,
    plugin_id: &str,
    now: i64,
) -> Result<String, PluginsError> {
    let not_found = || PluginsError::NotFound(plugin_id.to_owned());
    let opened = content::open_assignment(conn, actor, assignment_id)
        .await
        .map_err(|error| match error {
            OpenError::NotFound => not_found(),
            OpenError::Teaching(error) => PluginsError::Teaching(error),
            OpenError::Db(error) => PluginsError::Db(error),
        })?;
    let install = repo::find_install(conn, plugin_id)
        .await?
        .filter(|install| install.status == InstallStatus::Enabled)
        .filter(|install| install.ui.iter().any(|page| page.slot == ASSIGNMENT_TAB))
        .ok_or_else(not_found)?;
    let secret = derive_secret(key, install.id, install.secret_version)?;
    let claims = LaunchClaims {
        plugin: &install.plugin_id,
        assignment: assignment_id,
        user: actor.user_id(),
        // The role in this course, not the institution role: a teacher is a student of another
        // teacher's course.
        role: if opened.access.is_student() {
            "student"
        } else {
            "staff"
        },
        issued_at: now,
    };
    Ok(launch_token(&secret, &claims)?)
}

/// Stores the plugin's report on a submitted attempt, replacing its earlier one.
pub async fn put_report(
    conn: &mut PgConnection,
    caller: &PluginCaller,
    submission_id: Uuid,
    summary: &str,
    data: &Value,
) -> Result<(), PluginsError> {
    access::authorize_plugin(caller, Scope::ReportsWrite)?;
    repo::upsert_report(conn, caller.install_id(), submission_id, summary, data).await?;
    Ok(())
}

/// The reports on an attempt, for someone who may see the attempt as staff do.
pub async fn reports(
    conn: &mut PgConnection,
    actor: &Actor,
    submission_id: Uuid,
) -> Result<Vec<ReportRow>, PluginsError> {
    assessment::authorize_view_submission(conn, actor, submission_id).await?;
    Ok(repo::reports_of(conn, submission_id).await?)
}

#[cfg(test)]
mod tests;
