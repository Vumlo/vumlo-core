//! The course content tree: modules, one level of sub-modules, `text` items and assignments
//! (ADR-0020, ADR-0021).

mod document;
mod model;
mod repo;
mod routes;
mod service;

pub use document::{
    DocumentError, DocumentErrors, EditorConfigError, EffectiveProfiles, ValidDocument, limits,
    plain_text, validate_document, validate_document_within,
};
pub use model::{
    Assignment, AssignmentSummary, AssignmentType, BuiltinType, ContentModule, ContentSubModule,
    ContentTitle, Item, ItemKind, ItemPatch, ItemStatus, ItemSummary, MaxPoints, ModelError,
    Module, ModulePatch, ModuleStatus, OnDeadline, OpenedAssignment, SettingProblem, SettingsPatch,
    SubmissionSettings, editor_features,
};
pub use routes::router;
pub use service::{
    ContentError, CourseSummary, ItemRules, OpenError, copy_course_content, course_summary,
    create_item, create_module, delete_item, delete_module, editor_profiles, get_item, get_tree,
    lock_assignment_shared, lock_course_assignments, open_assignment, publish_item, publish_module,
    reorder_items, reorder_modules, update_item, update_module,
};
