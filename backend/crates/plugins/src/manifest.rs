//! `GET {base}/manifest`: what a plugin says about itself, checked against what this core accepts.
//! The core trusts none of it: an unknown scope, event or slot stops the install, and the message
//! names the value.

use std::collections::BTreeMap;

use access::{CourseRole, Scope};
use common::Role;
use serde::Deserialize;

use crate::PluginsError;
use crate::model::{
    API_VERSION, COURSE_SEGMENT, PluginManifest, ROUTE_METHODS, RouteAuth, RouteSpec, SLOTS,
    SUBSCRIBABLE, UiSlot, is_plain_segment,
};
use crate::settings::{MAX_SETTINGS, SettingSpec, is_key};

const MAX_SUBSCRIPTIONS: usize = 10;
const MAX_SLOTS: usize = 4;
const MAX_TITLE_CHARS: usize = 60;
const MAX_PATH_CHARS: usize = 200;
const MAX_ROUTES: usize = 20;

/// The manifest as a plugin writes it. Unknown fields are ignored (a plugin may say more about
/// itself); the known ones are all checked.
#[derive(Deserialize)]
struct Raw {
    id: String,
    version: String,
    api: String,
    #[serde(default)]
    subscribes: Vec<String>,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    ui: Vec<RawSlot>,
    #[serde(default)]
    settings: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    routes: Vec<RawRoute>,
}

#[derive(Deserialize)]
struct RawRoute {
    method: String,
    path: String,
    #[serde(default)]
    auth: Option<String>,
    #[serde(default)]
    roles: Vec<String>,
    #[serde(default)]
    course_roles: Vec<String>,
}

#[derive(Deserialize)]
struct RawSlot {
    slot: String,
    title: String,
    path: String,
}

fn invalid(message: impl Into<String>) -> PluginsError {
    PluginsError::Manifest(message.into())
}

/// A value from the plugin, cut short for a message.
fn shown(value: &str) -> String {
    let short: String = value.chars().take(40).collect();
    if short.len() < value.len() {
        format!("`{short}…`")
    } else {
        format!("`{short}`")
    }
}

fn is_slug(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && id.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

pub fn parse_manifest(bytes: &[u8]) -> Result<PluginManifest, PluginsError> {
    let raw: Raw = serde_json::from_slice(bytes)
        .map_err(|e| invalid(format!("not the JSON object of the protocol ({e})")))?;
    if !is_slug(&raw.id) {
        return Err(invalid(format!(
            "id {} must be lowercase letters, digits, `_` and `-`, start with a letter, and have at most 32 characters",
            shown(&raw.id)
        )));
    }
    if raw.version.is_empty()
        || raw.version.chars().count() > 32
        || raw.version.chars().any(char::is_control)
    {
        return Err(invalid("version must be 1 to 32 characters"));
    }
    if raw.api != API_VERSION {
        return Err(invalid(format!(
            "api {} is not supported; this core speaks api `{API_VERSION}`",
            shown(&raw.api)
        )));
    }
    if raw.subscribes.len() > MAX_SUBSCRIPTIONS {
        return Err(invalid(format!(
            "subscribes lists more than {MAX_SUBSCRIPTIONS} events"
        )));
    }
    let mut subscribes = Vec::new();
    for event in &raw.subscribes {
        if !SUBSCRIBABLE.contains(&event.as_str()) {
            return Err(invalid(format!(
                "cannot subscribe to {}; the events a plugin may subscribe to are {}",
                shown(event),
                SUBSCRIBABLE.join(", ")
            )));
        }
        if !subscribes.contains(event) {
            subscribes.push(event.clone());
        }
    }
    let mut scopes = Vec::new();
    for text in &raw.scopes {
        let scope: Scope = text.parse().map_err(|_| {
            invalid(format!(
                "scope {} is unknown; the scopes are {}",
                shown(text),
                Scope::ALL.map(Scope::as_str).join(", ")
            ))
        })?;
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    if raw.ui.len() > MAX_SLOTS {
        return Err(invalid(format!("ui lists more than {MAX_SLOTS} pages")));
    }
    let ui = raw
        .ui
        .into_iter()
        .map(slot)
        .collect::<Result<Vec<_>, _>>()?;
    // The web app names a tab by the plugin, so one plugin has one page in a slot.
    if let Some(twice) = ui
        .iter()
        .enumerate()
        .find(|(at, page)| ui[..*at].iter().any(|before| before.slot == page.slot))
        .map(|(_, page)| page)
    {
        return Err(invalid(format!(
            "ui slot {} has two pages; a plugin offers one page in a slot",
            shown(&twice.slot)
        )));
    }
    let settings = settings(raw.settings)?;
    let routes = routes(raw.routes)?;
    Ok(PluginManifest {
        id: raw.id,
        version: raw.version,
        subscribes,
        scopes,
        ui,
        settings,
        routes,
    })
}

fn routes(raw: Vec<RawRoute>) -> Result<Vec<RouteSpec>, PluginsError> {
    if raw.len() > MAX_ROUTES {
        return Err(invalid(format!(
            "routes lists more than {MAX_ROUTES} routes"
        )));
    }
    let mut found: Vec<RouteSpec> = Vec::new();
    for route in raw.into_iter().map(route) {
        let route = route?;
        if found
            .iter()
            .any(|before| before.method == route.method && before.path == route.path)
        {
            return Err(invalid(format!(
                "route {} {} is declared twice",
                route.method,
                shown(&route.path)
            )));
        }
        found.push(route);
    }
    Ok(found)
}

/// A name of a path parameter: lowercase letters and `_`.
fn is_param(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 20
        && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

fn route(raw: RawRoute) -> Result<RouteSpec, PluginsError> {
    let method = raw.method.to_ascii_uppercase();
    if !ROUTE_METHODS.contains(&method.as_str()) {
        return Err(invalid(format!(
            "route method {} is not one of {}",
            shown(&raw.method),
            ROUTE_METHODS.join(", ")
        )));
    }
    let path = raw.path;
    let at = |what: &str| invalid(format!("route {method} {}: {what}", shown(&path)));
    if !path.starts_with('/') || path.chars().count() > MAX_PATH_CHARS {
        return Err(at(
            "the path must start with `/` and be at most 200 characters",
        ));
    }
    // Every segment is a plain name or `{name}`.
    if path == "/" {
        return Err(at("the path needs at least one segment"));
    }
    let mut names: Vec<&str> = Vec::new();
    for segment in path.split('/').skip(1) {
        match segment.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            Some(name) if is_param(name) && !names.contains(&name) => names.push(name),
            Some(_) => {
                return Err(at(
                    "a `{name}` segment holds lowercase letters and `_`, once",
                ));
            }
            None if is_plain_segment(segment) => {}
            None => {
                return Err(at(
                    "a segment holds letters, digits, `.`, `_`, `~` and `-` only",
                ));
            }
        }
    }
    let auth = match raw.auth.as_deref() {
        None => RouteAuth::Session,
        Some(text) => text
            .parse()
            .map_err(|_| at(&format!("auth {} is not session or none", shown(text))))?,
    };
    if auth == RouteAuth::None {
        // Nobody is signed in, so there is no role to ask for and no course to resolve; and a call
        // that anyone may make must not change anything.
        if method != "GET" {
            return Err(at("a public route (`auth = \"none\"`) is GET only"));
        }
        if !raw.roles.is_empty() || !raw.course_roles.is_empty() {
            return Err(at("a public route has no roles"));
        }
        if names.contains(&COURSE_SEGMENT) {
            return Err(at("a public route has no `{course_id}` segment"));
        }
        return Ok(RouteSpec {
            method,
            auth,
            path,
            roles: Vec::new(),
            course_roles: Vec::new(),
        });
    }
    if raw.roles.is_empty() {
        return Err(at("roles must name at least one institution role"));
    }
    let mut roles: Vec<Role> = Vec::new();
    for text in &raw.roles {
        let role: Role = text.parse().map_err(|_| {
            at(&format!(
                "role {} is not admin, teacher or student",
                shown(text)
            ))
        })?;
        if !roles.contains(&role) {
            roles.push(role);
        }
    }
    let mut course_roles: Vec<CourseRole> = Vec::new();
    for text in &raw.course_roles {
        let role: CourseRole = text.parse().map_err(|_| {
            at(&format!(
                "course role {} is not teacher, assistant or student",
                shown(text)
            ))
        })?;
        if !course_roles.contains(&role) {
            course_roles.push(role);
        }
    }
    if !course_roles.is_empty() && !names.contains(&COURSE_SEGMENT) {
        return Err(at("course_roles need a `{course_id}` segment in the path"));
    }
    Ok(RouteSpec {
        method,
        auth,
        path,
        roles,
        course_roles,
    })
}

fn settings(
    raw: BTreeMap<String, serde_json::Value>,
) -> Result<BTreeMap<String, SettingSpec>, PluginsError> {
    if raw.len() > MAX_SETTINGS {
        return Err(invalid(format!(
            "settings lists more than {MAX_SETTINGS} settings"
        )));
    }
    raw.into_iter()
        .map(|(key, spec)| {
            if !is_key(&key) {
                return Err(invalid(format!(
                    "setting {} must be lowercase letters, digits and `_`, start with a letter, and have at most 40 characters",
                    shown(&key)
                )));
            }
            let spec: SettingSpec = serde_json::from_value(spec)
                .map_err(|e| invalid(format!("setting `{key}` is not valid ({e})")))?;
            spec.validate(&key).map_err(invalid)?;
            Ok((key, spec))
        })
        .collect()
}

fn slot(raw: RawSlot) -> Result<UiSlot, PluginsError> {
    if !SLOTS.contains(&raw.slot.as_str()) {
        return Err(invalid(format!(
            "ui slot {} is unknown; the slots are {}",
            shown(&raw.slot),
            SLOTS.join(", ")
        )));
    }
    let title = raw.title.trim();
    if title.is_empty()
        || title.chars().count() > MAX_TITLE_CHARS
        || title.chars().any(char::is_control)
    {
        return Err(invalid(format!(
            "the title of ui slot {} must be 1 to {MAX_TITLE_CHARS} characters",
            shown(&raw.slot)
        )));
    }
    let path = &raw.path;
    // A path on the plugin's own origin: not another host (`//host`), no `..`, nothing a browser
    // would read as a scheme or a space.
    let plain = path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains("..")
        && !path.contains('\\')
        && path.chars().count() <= MAX_PATH_CHARS
        && path.chars().all(|c| c.is_ascii_graphic());
    if !plain {
        return Err(invalid(format!(
            "the path {} of ui slot {} must start with a single `/` and hold plain characters only",
            shown(path),
            shown(&raw.slot)
        )));
    }
    Ok(UiSlot {
        slot: raw.slot,
        title: title.to_owned(),
        path: raw.path,
    })
}

#[cfg(test)]
mod tests;
