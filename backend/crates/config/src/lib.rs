//! The configuration of a Vumlo installation: one schema for `vumlo.toml`, read the same way by
//! the server and by `vumlo-cli` (ADR-0054).

mod client;
mod load;
mod modules;
mod schema;
mod settings;
mod ts;

pub use client::{ClientConfig, PublicConfig, PublicInstitution, PublicServer, api_url};
pub use load::{
    Error, FileSource, Loaded, Options, Sources, find_file, load, load_process, parse_env_file,
    parse_overrides,
};
pub use modules::{
    KnownModule, ModuleEntry, ModuleStatus, Modules, RESERVED_KEYS, ResolvedModule, SettingKind,
    SettingSpec, check_endpoint, known, known_modules, resolve as resolve_modules,
};
pub use schema::{Visibility, env_name, public_paths, secret_paths, settings_schema};
pub use settings::*;
pub use ts::client_config_types;

#[cfg(test)]
mod tests;
