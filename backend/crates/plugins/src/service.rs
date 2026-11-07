//! Business rules. A function that acts for a plugin authorizes its scope first; the install
//! functions are for the administrator CLI, which has no `Actor` and audits what it changes
//! (source `cli`). Nothing here begins or commits a transaction.

use std::time::Duration;

use access::{self, Actor};
use std::collections::BTreeMap;

use access::{CourseScope, PluginCaller, Scope};
use events::{Event, OutboxEvent};
use learning::assessment;
use learning::content::{self, OpenError};
use learning::teaching;
use module_api::OutboundClient;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::PluginsError;
use crate::manifest::parse_manifest;
use crate::model::{
    ASSIGNMENT_TAB, Install, InstallStatus, InstallSummary, Installed, PluginManifest, PluginToken,
    SETTINGS_CHANGED,
};
use crate::repo::{self, NewInstall, ReportRow, scope_names};
use crate::secret::{LaunchClaims, SecretKey, derive_secret, launch_token, open, seal};
use crate::settings::{SettingScope, SettingSpec};

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
    courses: &CourseScope,
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
            courses,
            ui_slots: Value::Array(ui),
            settings: &manifest.settings,
            token_hash: &token.hash(),
        },
    )
    .await;
    if let Err(error) = inserted {
        return Err(
            match error.as_database_error().and_then(|e| e.code()).as_deref() {
                Some("23505") => PluginsError::AlreadyInstalled(manifest.id.clone()),
                Some("23503") => PluginsError::CourseGone,
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

/// Replaces the granted scopes and the courses the plugin may reach. The scopes stay within what
/// the plugin asked for; a course that is not there is `CourseGone`.
pub async fn grant(
    conn: &mut PgConnection,
    plugin_id: &str,
    scopes: &[Scope],
    courses: &CourseScope,
) -> Result<Install, PluginsError> {
    let install = get(conn, plugin_id).await?;
    if let Some(extra) = scopes
        .iter()
        .find(|s| !install.requested_scopes.contains(s))
    {
        return Err(PluginsError::NotRequested(extra.as_str().to_owned()));
    }
    repo::set_scopes(conn, install.id, &scope_names(&unique(scopes))).await?;
    repo::set_courses(conn, install.id, courses)
        .await
        .map_err(
            |error| match error.as_database_error().and_then(|e| e.code()).as_deref() {
                Some("23503") => PluginsError::CourseGone,
                _ => PluginsError::Db(error),
            },
        )?;
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
    // A change of settings is for the plugin it is about, not for everyone who subscribed to it.
    let only = (event.kind == SETTINGS_CHANGED).then(|| {
        event.payload["install_id"]
            .as_str()
            .and_then(|id| id.parse().ok())
            .unwrap_or(Uuid::nil())
    });
    // A plugin is sent the events of the courses it may reach and no others (ADR-0074). An event
    // without a course (a change of settings) is not about one.
    let course = event.payload["course_id"]
        .as_str()
        .and_then(|id| id.parse().ok());
    for install_id in repo::enabled_subscribers(conn, &event.kind, course).await? {
        if only.is_some_and(|only| only != install_id) {
            continue;
        }
        if repo::insert_delivery(
            conn,
            install_id,
            event.id,
            &event.kind,
            &event.payload,
            event.occurred_at,
        )
        .await?
        {
            owed += 1;
        }
    }
    Ok(owed)
}

// ---------------------------------------------------------------------------------------------
// Settings (ADR-0072)
// ---------------------------------------------------------------------------------------------

/// One setting in one scope, as an administrator sees it. A secret never shows its value.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingView {
    pub key: String,
    pub spec: SettingSpec,
    /// Whether a value was set in this scope; if not, the default applies.
    pub is_set: bool,
    /// The value in force. `None` for a secret whatever it is, and when there is no value and no
    /// default.
    pub value: Option<Value>,
}

fn scope_of(course: Option<Uuid>) -> SettingScope {
    if course.is_some() {
        SettingScope::Course
    } else {
        SettingScope::Institution
    }
}

/// The spec of `name`, which must live in the scope that `course` implies.
fn spec_in_scope<'a>(
    install: &'a Install,
    name: &str,
    course: Option<Uuid>,
) -> Result<&'a SettingSpec, PluginsError> {
    let spec = install
        .settings
        .get(name)
        .ok_or_else(|| PluginsError::NoSuchSetting {
            plugin: install.plugin_id.clone(),
            key: name.to_owned(),
        })?;
    if spec.scope != scope_of(course) {
        return Err(PluginsError::WrongScope {
            key: name.to_owned(),
            scope: spec.scope.as_str(),
            hint: match spec.scope {
                SettingScope::Course => "give the course",
                SettingScope::Institution => "it belongs to no course",
            },
        });
    }
    Ok(spec)
}

async fn stored(
    conn: &mut PgConnection,
    install: &Install,
    course: Option<Uuid>,
) -> Result<BTreeMap<String, Value>, PluginsError> {
    Ok(repo::settings_of(conn, install.id, course)
        .await?
        .into_iter()
        .map(|row| (row.key, row.value))
        .collect())
}

fn view(key: &str, spec: &SettingSpec, stored: &BTreeMap<String, Value>) -> SettingView {
    let value = stored.get(key);
    SettingView {
        key: key.to_owned(),
        spec: spec.clone(),
        is_set: value.is_some(),
        value: if spec.secret {
            None
        } else {
            value.or(spec.default.as_ref()).cloned()
        },
    }
}

/// The settings of the plugin that live in one scope: the institution's, or those of `course`.
pub async fn list_settings(
    conn: &mut PgConnection,
    plugin_id: &str,
    course: Option<Uuid>,
) -> Result<Vec<SettingView>, PluginsError> {
    let install = get(conn, plugin_id).await?;
    let stored = stored(conn, &install, course).await?;
    Ok(install
        .settings
        .iter()
        .filter(|(_, spec)| spec.scope == scope_of(course))
        .map(|(key, spec)| view(key, spec, &stored))
        .collect())
}

pub async fn get_setting(
    conn: &mut PgConnection,
    plugin_id: &str,
    name: &str,
    course: Option<Uuid>,
) -> Result<SettingView, PluginsError> {
    let install = get(conn, plugin_id).await?;
    let spec = spec_in_scope(&install, name, course)?;
    Ok(view(name, spec, &stored(conn, &install, course).await?))
}

/// Stores a value after checking it against the spec the plugin declared, and tells the plugin
/// (`plugin.settings.changed`) in the same transaction. A secret is stored sealed. Returns whether
/// anything changed: the value it already has is left alone and announces nothing.
pub async fn set_setting(
    conn: &mut PgConnection,
    key: &SecretKey,
    plugin_id: &str,
    name: &str,
    value: Value,
    course: Option<Uuid>,
) -> Result<bool, PluginsError> {
    let install = get(conn, plugin_id).await?;
    let spec = spec_in_scope(&install, name, course)?;
    spec.check(&value).map_err(|why| PluginsError::BadSetting {
        key: name.to_owned(),
        why,
    })?;
    let current = stored(conn, &install, course).await?.remove(name);
    let to_store = match (spec.secret, &value) {
        (true, Value::String(plain)) => {
            // Compared as typed: the sealed form differs every time.
            if let Some(Value::String(sealed)) = &current
                && open(key, install.id, name, sealed).is_ok_and(|was| &was == plain)
            {
                return Ok(false);
            }
            Value::String(seal(key, install.id, name, plain)?)
        }
        _ if current.as_ref() == Some(&value) => return Ok(false),
        _ => value,
    };
    repo::upsert_setting(conn, install.id, course, name, &to_store)
        .await
        .map_err(
            |error| match error.as_database_error().and_then(|e| e.code()).as_deref() {
                Some("23503") => PluginsError::CourseGone,
                _ => PluginsError::Db(error),
            },
        )?;
    changed(conn, &install, course).await?;
    Ok(true)
}

/// Removes the value, so that the default applies again. Returns whether there was one.
pub async fn unset_setting(
    conn: &mut PgConnection,
    plugin_id: &str,
    name: &str,
    course: Option<Uuid>,
) -> Result<bool, PluginsError> {
    let install = get(conn, plugin_id).await?;
    spec_in_scope(&install, name, course)?;
    let removed = repo::delete_setting(conn, install.id, course, name).await?;
    if removed {
        changed(conn, &install, course).await?;
    }
    Ok(removed)
}

async fn changed(
    conn: &mut PgConnection,
    install: &Install,
    course: Option<Uuid>,
) -> Result<(), PluginsError> {
    events::emit(
        conn,
        Event::PluginSettingsChanged {
            install_id: install.id,
            course_id: course,
        },
    )
    .await?;
    Ok(())
}

/// The values the plugin reads with `settings:read`: those of the institution and, when `course` is
/// given, those of that course (a course that is not there is `CourseGone`). A setting that has
/// neither a value nor a default is left out. This is the one place a secret leaves the core, and
/// it leaves it for the plugin it belongs to.
pub async fn settings_for_plugin(
    conn: &mut PgConnection,
    caller: &PluginCaller,
    key: &SecretKey,
    course: Option<Uuid>,
) -> Result<Map<String, Value>, PluginsError> {
    access::authorize_plugin(caller, Scope::SettingsRead)?;
    if let Some(course) = course {
        access::authorize_plugin_course(caller, Scope::SettingsRead, course)?;
    }
    if let Some(course) = course
        && !teaching::course_exists(conn, course).await?
    {
        return Err(PluginsError::CourseGone);
    }
    let install = get(conn, caller.plugin_id()).await?;
    let mut values = Map::new();
    for scope in [None, course] {
        let stored = stored(conn, &install, scope).await?;
        for (name, spec) in install
            .settings
            .iter()
            .filter(|(_, spec)| spec.scope == scope_of(scope))
        {
            let value = match (stored.get(name), spec.secret) {
                (Some(Value::String(sealed)), true) => {
                    Some(Value::String(open(key, install.id, name, sealed)?))
                }
                (Some(value), _) => Some(value.clone()),
                (None, _) => spec.default.clone(),
            };
            if let Some(value) = value {
                values.insert(name.clone(), value);
            }
        }
        if course.is_none() {
            break;
        }
    }
    Ok(values)
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
        .filter(|install| install.courses.allows(opened.course_id))
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
    repo::upsert_report(conn, caller.install_id(), submission_id, summary, data)
        .await
        .map_err(
            |error| match error.as_database_error().and_then(|e| e.code()).as_deref() {
                // The foreign key to the attempt: it is not there (any more).
                Some("23503") => PluginsError::SubmissionGone,
                _ => PluginsError::Db(error),
            },
        )?;
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
