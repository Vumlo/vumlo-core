//! `GET {base}/manifest`: what a plugin says about itself, checked against what this core accepts.
//! The core trusts none of it: an unknown scope, event or slot stops the install, and the message
//! names the value.

use access::Scope;
use serde::Deserialize;

use crate::PluginsError;
use crate::model::{API_VERSION, PluginManifest, SLOTS, SUBSCRIBABLE, UiSlot};

const MAX_SUBSCRIPTIONS: usize = 10;
const MAX_SLOTS: usize = 4;
const MAX_TITLE_CHARS: usize = 60;
const MAX_PATH_CHARS: usize = 200;

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
    Ok(PluginManifest {
        id: raw.id,
        version: raw.version,
        subscribes,
        scopes,
        ui,
    })
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
