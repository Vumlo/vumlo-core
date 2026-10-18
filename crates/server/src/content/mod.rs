//! The course content tree: modules, one level of sub-modules, `text` items and assignments
//! (ADR-0020, ADR-0021).

mod document;
mod model;
mod repo;
mod routes;
mod service;
mod types;

pub use document::{
    DocumentError, DocumentErrors, EditorConfigError, EffectiveProfiles, limits, plain_text,
    validate_document, validate_document_within,
};
pub use model::{
    Assignment, AssignmentSummary, AssignmentType, BuiltinType, ContentModule, ContentSubModule,
    ContentTitle, Hit, HitKind, Item, ItemKind, ItemPatch, ItemStatus, ItemSummary, MaxPoints,
    ModelError, Module, ModulePatch, ModuleStatus, MyWork, OnDeadline, OpenedAssignment,
    SettingProblem, SettingsPatch, SubmissionSettings, editor_features,
};
pub use routes::router;
pub use service::{
    ContentError, CourseSummary, ItemRules, OpenError, copy_course_content, course_summary,
    create_item, create_module, delete_item, delete_module, editor_profiles, get_item,
    get_revision, get_tree, item_meta, lock_assignment_shared, lock_course_assignments, my_work,
    open_assignment, publish_item, publish_module, publish_revision, reorder_items,
    reorder_modules, search, update_item, update_module,
};
pub use types::form::Holders;
pub use types::quiz::ResultQuestion;
pub use types::quiz::result_questions as quiz_result_questions;
pub use types::{AutoGrade, Claims, Stage, ValidContent};
