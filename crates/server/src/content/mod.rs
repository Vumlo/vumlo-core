//! The course content tree: modules, one level of sub-modules and lessons (ADR-0020).

mod model;
mod repo;
mod routes;
mod service;

pub use model::{
    ContentModule, ContentSubModule, ContentTitle, Item, ItemKind, ItemPatch, ItemStatus,
    ItemSummary, ModelError, Module, ModulePatch, ModuleStatus,
};
pub use routes::router;
pub use service::{
    ContentError, create_item, create_module, delete_item, delete_module, get_item, get_tree,
    publish_item, publish_module, reorder_items, reorder_modules, update_item, update_module,
};
