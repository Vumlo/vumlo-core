//! The registry of activity types (ADR-0097). The composition root builds a [`Types`] from what
//! each crate offers and installs it once; it never changes afterwards, so a [`Kind`] taken from
//! it is valid for the life of the process. The stored form of a type is its id, as `text`.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Deref;
use std::sync::OnceLock;

use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::postgres::{PgArgumentBuffer, PgTypeInfo, PgValueRef};
use sqlx::{Decode, Encode, Postgres, Type};

use crate::{ActivityType, Capabilities};

/// Why a set of types could not be built or installed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstallError {
    #[error("a registry of activity types is already installed")]
    AlreadyInstalled,
    #[error("the activity type id `{0}` is registered twice")]
    DuplicateId(String),
    #[error("`{0}` is not an activity type id (a letter, then letters, digits or `_`, 32 at most)")]
    BadId(String),
}

/// Why an id did not give a type.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("no registry of activity types is installed")]
    NotInstalled,
    #[error("unknown activity type `{0}`")]
    Unknown(String),
}

/// One activity type of the installed registry. Equality and `Debug` go by its id.
#[derive(Clone, Copy)]
pub struct Kind(&'static dyn ActivityType);

impl Kind {
    pub fn id(&self) -> &'static str {
        self.0.id()
    }
}

impl Deref for Kind {
    type Target = dyn ActivityType;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl PartialEq for Kind {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl Eq for Kind {}

impl fmt::Debug for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Kind").field(&self.id()).finish()
    }
}

impl Type<Postgres> for Kind {
    fn type_info() -> PgTypeInfo {
        <str as Type<Postgres>>::type_info()
    }

    fn compatible(ty: &PgTypeInfo) -> bool {
        <str as Type<Postgres>>::compatible(ty)
    }
}

/// Reads the id and looks it up in the installed registry: a stored id this process does not know
/// is an error naming it, not a panic.
impl<'r> Decode<'r, Postgres> for Kind {
    fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
        let id = <&str as Decode<Postgres>>::decode(value)?;
        get(id).map_err(Into::into)
    }
}

impl Encode<'_, Postgres> for Kind {
    fn encode_by_ref(&self, buf: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
        <&str as Encode<Postgres>>::encode_by_ref(&self.id(), buf)
    }
}

/// The types to install, by id. Built by the composition root, and by tests that need no
/// installed registry: [`Types::get`], [`Types::available`] and [`Types::ids_where`] are what the
/// free functions run on the installed one.
#[derive(Default)]
pub struct Types(BTreeMap<&'static str, &'static dyn ActivityType>);

/// `^[a-z][a-z0-9_]{0,31}$`
fn is_valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && id.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

impl Types {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `t`, refusing an id that is malformed or already taken.
    pub fn with(mut self, t: &'static dyn ActivityType) -> Result<Self, InstallError> {
        let id = t.id();
        if !is_valid_id(id) {
            return Err(InstallError::BadId(id.to_owned()));
        }
        if self.0.insert(id, t).is_some() {
            return Err(InstallError::DuplicateId(id.to_owned()));
        }
        Ok(self)
    }

    pub fn get(&self, id: &str) -> Result<Kind, RegistryError> {
        self.0
            .get(id)
            .map(|t| Kind(*t))
            .ok_or_else(|| RegistryError::Unknown(id.to_owned()))
    }

    /// Every id, sorted.
    pub fn available(&self) -> Vec<&'static str> {
        self.0.keys().copied().collect()
    }

    /// The ids of the types whose capabilities satisfy `f`, sorted, as owned text to bind as a
    /// `text[]`.
    pub fn ids_where(&self, f: fn(Capabilities) -> bool) -> Vec<String> {
        self.0
            .iter()
            .filter(|(_, t)| f(t.capabilities()))
            .map(|(id, _)| (*id).to_owned())
            .collect()
    }
}

static REGISTRY: OnceLock<Types> = OnceLock::new();

/// Installs the registry of this process. Called once by the composition root; a second call is
/// an error, because a registry that could change would make a stored id mean different things.
pub fn install(types: Types) -> Result<(), InstallError> {
    REGISTRY
        .set(types)
        .map_err(|_| InstallError::AlreadyInstalled)
}

/// The type of an id in the installed registry.
pub fn get(id: &str) -> Result<Kind, RegistryError> {
    REGISTRY.get().ok_or(RegistryError::NotInstalled)?.get(id)
}

/// Every installed id, sorted. Empty, not an error, when no registry is installed: `get` is the
/// call that tells a process which forgot to install one.
pub fn available() -> Vec<&'static str> {
    REGISTRY.get().map(Types::available).unwrap_or_default()
}

/// The installed ids whose capabilities satisfy `f`, sorted and owned, to bind as a `text[]`.
/// Empty when no registry is installed.
pub fn ids_where(f: fn(Capabilities) -> bool) -> Vec<String> {
    REGISTRY
        .get()
        .map(|types| types.ids_where(f))
        .unwrap_or_default()
}

/// Installs `build()` unless a registry is already there. For test binaries only, where many
/// tests of one process start the application.
#[cfg(feature = "testing")]
pub fn ensure_installed(build: impl FnOnce() -> Types) {
    REGISTRY.get_or_init(build);
}

#[cfg(test)]
mod tests;
