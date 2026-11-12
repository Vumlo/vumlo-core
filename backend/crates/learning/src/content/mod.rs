//! The course content tree: modules, one level of sub-modules, `text` items and assignments
//! (ADR-0020, ADR-0021).

mod document;
mod model;
mod repo;
mod routes;
mod rules;
mod service;
mod standing;
mod types;

pub use document::{
    DocumentError, DocumentErrors, EditorConfigError, EffectiveProfiles, FileRef, distinct_ids,
    limits, plain_text, validate_document, validate_document_within,
};
pub use model::{
    Access, Assignment, AssignmentSummary, AssignmentType, BuiltinType, ConditionInput,
    ConditionKind, ContentModule, ContentSubModule, ContentTitle, DraftItem, FactScope, Hit,
    HitKind, Item, ItemFacts, ItemKind, ItemPatch, ItemStatus, ItemSummary, MaxPoints, ModelError,
    Module, ModulePatch, ModuleStatus, NextDeadline, OnDeadline, OpenAssignment, OpenedAssignment,
    SettingProblem, SettingsPatch, SourceFact, Standing, SubmissionSettings, Unmet,
    editor_features,
};
pub use repo::CopiedIds;
pub use routes::router;
pub use rules::{Condition, Node};
pub use service::{
    ContentError, CourseSummary, ItemRules, OpenError, announce_visible, authorize_edit_item,
    copy_course_content, course_summary, create_file_item, create_item, create_module, delete_item,
    delete_module, draft_items, editor_profiles, get_item, get_revision, get_tree, item_meta,
    lock_assignment_shared, lock_course_assignments, lock_facts, mark_done, may_read_item_files,
    next_deadlines, open_assignment, open_assignments, publish_item, publish_module,
    publish_revision, reorder_items, reorder_modules, replace_source_facts, search, sees_drafts,
    set_due_at, unmark_done, update_item, update_module,
};
pub use types::file::check_names as check_file_names;
pub use types::form::Holders;
pub use types::quiz::ResultQuestion;
pub use types::quiz::result_questions as quiz_result_questions;
pub use types::{AutoGrade, Claims, Stage, ValidContent};
// Seeds the references of an item for the tests of `learning::files`.
#[cfg(test)]
pub use repo::replace_item_files;
