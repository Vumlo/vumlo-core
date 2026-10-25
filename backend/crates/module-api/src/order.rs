//! The order modules start in: every module after the ones it requires.

use std::collections::BTreeMap;

use crate::Module;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OrderError {
    #[error(
        "module id `{0}` is not valid: lowercase letters, digits and `_`, starting with a letter"
    )]
    InvalidId(String),
    #[error("module `{0}` is compiled in twice")]
    Duplicate(String),
    #[error("module `{module}` requires `{missing}`, which is not compiled into this binary")]
    Missing { module: String, missing: String },
    #[error("modules require each other in a cycle: {}", .0.join(", "))]
    Cycle(Vec<String>),
}

/// Sorts `modules` so that each comes after everything it `requires`. Deterministic: of the modules
/// that are ready, the one with the smallest id goes first.
pub fn init_order<'a>(modules: &[&'a dyn Module]) -> Result<Vec<&'a dyn Module>, OrderError> {
    let mut by_id: BTreeMap<&str, &'a dyn Module> = BTreeMap::new();
    for module in modules {
        let manifest = module.manifest();
        if !manifest.id_is_valid() {
            return Err(OrderError::InvalidId(manifest.id.to_owned()));
        }
        if by_id.insert(manifest.id, *module).is_some() {
            return Err(OrderError::Duplicate(manifest.id.to_owned()));
        }
    }
    for (id, module) in &by_id {
        if let Some(missing) = module
            .manifest()
            .requires
            .iter()
            .find(|required| !by_id.contains_key(**required))
        {
            return Err(OrderError::Missing {
                module: (*id).to_owned(),
                missing: (*missing).to_owned(),
            });
        }
    }
    let mut ordered: Vec<&'a dyn Module> = Vec::with_capacity(by_id.len());
    let mut placed: Vec<&str> = Vec::with_capacity(by_id.len());
    while !by_id.is_empty() {
        let next = by_id
            .iter()
            .find(|(_, module)| {
                module
                    .manifest()
                    .requires
                    .iter()
                    .all(|required| placed.contains(required))
            })
            .map(|(id, _)| *id);
        let Some(id) = next else {
            return Err(OrderError::Cycle(
                by_id.keys().map(|id| (*id).to_owned()).collect(),
            ));
        };
        if let Some(module) = by_id.remove(id) {
            placed.push(module.manifest().id);
            ordered.push(module);
        }
    }
    Ok(ordered)
}
