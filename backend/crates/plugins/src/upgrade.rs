//! Taking a plugin's new manifest (ADR-0079). An administrator reads what changed and approves only
//! what is new to approve: a scope the plugin did not ask for before. Everything else follows the
//! manifest at once, because the plugin cannot do more with it than it could: events it asks for
//! are the ones it can already read, settings are values only an administrator sets, and a route
//! is checked against roles on every call. The token, the secret, the courses and the grants it
//! already has stay as they are; a grant for a scope it no longer asks for goes.

use access::Scope;
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::PluginsError;
use crate::model::{Install, PluginManifest};
use crate::repo::{self, ManifestUpdate, scope_names};
use crate::service;

/// What a new manifest changes about an install, without changing anything.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpgradePlan {
    pub from_version: String,
    pub to_version: String,
    /// Scopes the plugin asks for now and did not before: the only thing to approve.
    pub scopes_new: Vec<Scope>,
    /// Scopes it no longer asks for. A grant of one of them is revoked.
    pub scopes_dropped: Vec<Scope>,
    pub subscribes_added: Vec<String>,
    pub subscribes_removed: Vec<String>,
    pub settings_added: Vec<String>,
    pub settings_removed: Vec<String>,
    pub settings_changed: Vec<String>,
    /// `METHOD /path`.
    pub routes_added: Vec<String>,
    pub routes_removed: Vec<String>,
    pub routes_changed: Vec<String>,
    pub pages_changed: bool,
}

impl UpgradePlan {
    /// Whether the manifest says what the install already holds.
    pub fn is_empty(&self) -> bool {
        self.from_version == self.to_version
            && self.scopes_new.is_empty()
            && self.scopes_dropped.is_empty()
            && self.subscribes_added.is_empty()
            && self.subscribes_removed.is_empty()
            && self.settings_added.is_empty()
            && self.settings_removed.is_empty()
            && self.settings_changed.is_empty()
            && self.routes_added.is_empty()
            && self.routes_removed.is_empty()
            && self.routes_changed.is_empty()
            && !self.pages_changed
    }
}

fn difference<T: PartialEq + Clone>(of: &[T], without: &[T]) -> Vec<T> {
    of.iter()
        .filter(|x| !without.contains(x))
        .cloned()
        .collect()
}

fn sorted_scopes(scopes: &[Scope]) -> Vec<Scope> {
    let mut scopes = scopes.to_vec();
    scopes.sort();
    scopes.dedup();
    scopes
}

/// What `manifest` would change about `install`.
pub fn plan(install: &Install, manifest: &PluginManifest) -> UpgradePlan {
    let key = |r: &crate::model::RouteSpec| format!("{} {}", r.method, r.path);
    let old_routes: Vec<_> = install.routes.iter().map(key).collect();
    let new_routes: Vec<_> = manifest.routes.iter().map(key).collect();
    let routes_changed = manifest
        .routes
        .iter()
        .filter(|new| {
            install
                .routes
                .iter()
                .any(|old| key(old) == key(new) && old != *new)
        })
        .map(key)
        .collect();
    let old_settings: Vec<String> = install.settings.keys().cloned().collect();
    let new_settings: Vec<String> = manifest.settings.keys().cloned().collect();
    UpgradePlan {
        from_version: install.version.clone(),
        to_version: manifest.version.clone(),
        scopes_new: sorted_scopes(&difference(&manifest.scopes, &install.requested_scopes)),
        scopes_dropped: sorted_scopes(&difference(&install.requested_scopes, &manifest.scopes)),
        subscribes_added: difference(&manifest.subscribes, &install.subscribes),
        subscribes_removed: difference(&install.subscribes, &manifest.subscribes),
        settings_added: difference(&new_settings, &old_settings),
        settings_removed: difference(&old_settings, &new_settings),
        settings_changed: manifest
            .settings
            .iter()
            .filter(|(name, spec)| install.settings.get(*name).is_some_and(|old| old != *spec))
            .map(|(name, _)| name.clone())
            .collect(),
        routes_added: difference(&new_routes, &old_routes),
        routes_removed: difference(&old_routes, &new_routes),
        routes_changed,
        pages_changed: install.ui != manifest.ui,
    }
}

/// A stored setting value the new manifest has no place for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedValue {
    pub key: String,
    /// `None`: the value for the whole institution.
    pub course: Option<Uuid>,
    pub why: &'static str,
}

#[derive(Debug)]
pub struct Upgraded {
    pub before: Install,
    pub after: Install,
    pub plan: UpgradePlan,
    pub dropped: Vec<DroppedValue>,
}

/// Why a stored value cannot stay, or `None` when it can. A secret is kept sealed only if the
/// setting is a secret before and after; anything else would show or lose it.
fn unfit(
    old: &Install,
    manifest: &PluginManifest,
    course: Option<Uuid>,
    key: &str,
    value: &Value,
) -> Option<&'static str> {
    let Some(spec) = manifest.settings.get(key) else {
        return Some("the plugin no longer declares it");
    };
    if (spec.scope == crate::SettingScope::Course) != course.is_some() {
        return Some("it now belongs to another scope");
    }
    let was_secret = old.settings.get(key).is_some_and(|old| old.secret);
    match (was_secret, spec.secret) {
        (true, true) => value
            .is_string()
            .then_some(())
            .map_or(Some("it is not sealed text"), |()| None),
        (false, false) => spec
            .check(value)
            .err()
            .map(|_| "it no longer meets the declaration"),
        (true, false) => Some("it is no longer a secret"),
        (false, true) => Some("it is now a secret and must be set again"),
    }
}

/// Makes the install what `manifest` says. `approved` are the new scopes the administrator granted;
/// each must be one the plugin newly asks for. Values that no longer fit are removed and reported,
/// and the plugin is told with `plugin.settings.changed` when its settings changed in any way.
pub async fn apply(
    conn: &mut PgConnection,
    plugin_id: &str,
    manifest: &PluginManifest,
    approved: &[Scope],
) -> Result<Upgraded, PluginsError> {
    let before = service::get(conn, plugin_id).await?;
    if manifest.id != before.plugin_id {
        return Err(PluginsError::Manifest(format!(
            "the manifest is of `{}`, not of `{}`",
            manifest.id, before.plugin_id
        )));
    }
    let plan = plan(&before, manifest);
    if let Some(extra) = approved.iter().find(|s| !plan.scopes_new.contains(s)) {
        return Err(PluginsError::NotRequested(extra.as_str().to_owned()));
    }
    // What it held and still asks for, and what was approved now.
    let mut granted: Vec<Scope> = before
        .scopes
        .iter()
        .copied()
        .filter(|s| manifest.scopes.contains(s))
        .chain(approved.iter().copied())
        .collect();
    granted.sort();
    granted.dedup();
    let requested = sorted_scopes(&manifest.scopes);
    repo::update_manifest(
        conn,
        before.id,
        ManifestUpdate {
            version: &manifest.version,
            subscribes: &manifest.subscribes,
            requested_scopes: &scope_names(&requested),
            scopes: &scope_names(&granted),
            ui_slots: service::ui_json(&manifest.ui),
            routes: repo::routes_json(&manifest.routes),
            settings: &manifest.settings,
        },
    )
    .await?;
    let mut dropped = Vec::new();
    for (course, key, value) in repo::all_settings(conn, before.id).await? {
        if let Some(why) = unfit(&before, manifest, course, &key, &value) {
            repo::delete_setting(conn, before.id, course, &key).await?;
            dropped.push(DroppedValue { key, course, why });
        }
    }
    let after = service::get(conn, plugin_id).await?;
    if !plan.settings_added.is_empty()
        || !plan.settings_removed.is_empty()
        || !plan.settings_changed.is_empty()
        || !dropped.is_empty()
    {
        service::settings_changed(conn, &after, None).await?;
    }
    Ok(Upgraded {
        before,
        after,
        plan,
        dropped,
    })
}

#[cfg(test)]
mod tests;
