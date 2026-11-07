//! The module's settings (ADR-0070), read from what `vumlo-config` resolved.

use vumlo_config::ResolvedModule;

use crate::persona::Persona;

/// Messages a user may send per day when `daily_cap` is not set.
const DEFAULT_DAILY_CAP: u32 = 100;
const DEFAULT_RETENTION_DAYS: u32 = 30;
const DEFAULT_ROLES: &str = "teacher,admin";

pub struct Config<'a>(&'a ResolvedModule);

impl<'a> Config<'a> {
    pub fn new(resolved: &'a ResolvedModule) -> Self {
        Self(resolved)
    }

    fn text(&self, key: &str) -> Option<&'a str> {
        self.0.settings.get(key)?.as_str()
    }

    fn int(&self, key: &str) -> Option<u32> {
        u32::try_from(self.0.settings.get(key)?.as_u64()?).ok()
    }

    /// The base URL of an OpenAI-compatible API, such as `https://api.openai.com/v1`.
    pub fn endpoint(&self) -> Option<&'a str> {
        self.text("endpoint")
    }

    pub fn model(&self) -> Option<&'a str> {
        self.text("model")
    }

    pub fn api_key(&self) -> Option<&'a str> {
        Some(self.0.secrets.get("api_key")?.expose())
    }

    /// The tokens of `roles` (`student`, `teacher`, `admin`) as written, trimmed and lowercased.
    pub fn role_tokens(&self) -> Vec<String> {
        self.text("roles")
            .unwrap_or(DEFAULT_ROLES)
            .split(',')
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect()
    }

    /// Whether the admin switched the Buddy on for this kind of user.
    pub fn allows(&self, persona: Persona) -> bool {
        self.role_tokens().iter().any(|t| t == persona.token())
    }

    /// Messages per user per day; `0` is no limit.
    pub fn daily_cap(&self) -> u32 {
        self.int("daily_cap").unwrap_or(DEFAULT_DAILY_CAP)
    }

    /// Days a chat is kept; at least one.
    pub fn retention_days(&self) -> u32 {
        self.int("retention_days")
            .unwrap_or(DEFAULT_RETENTION_DAYS)
            .max(1)
    }

    /// What the first open of the panel says about where messages go.
    pub fn notice(&self) -> Option<&'a str> {
        self.text("notice").filter(|n| !n.trim().is_empty())
    }
}

#[cfg(test)]
mod tests;
