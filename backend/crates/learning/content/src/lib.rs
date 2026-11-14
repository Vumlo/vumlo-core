//! The course content tree: modules, one level of sub-modules, `text` items and assignments
//! (ADR-0020, ADR-0021).

mod model;
mod points;
mod repo;
mod routes;
mod rules;
mod service;
mod standing;
mod state;
mod types;

pub use model::{
    Access, Assignment, AssignmentSummary, ConditionInput, ConditionKind, ContentModule,
    ContentSubModule, ContentTitle, DraftItem, FactScope, Hit, HitKind, Item, ItemFacts, ItemKind,
    ItemPatch, ItemStatus, ItemSummary, MaxPoints, ModelError, Module, ModulePatch, ModuleStatus,
    NextDeadline, OnDeadline, OpenAssignment, OpenedAssignment, SettingProblem, SettingsPatch,
    SourceFact, Standing, SubmissionSettings, TemplateCounts, Unmet,
};
pub use points::parse_decimal;
pub use repo::CopiedIds;
pub use routes::{api_item, item_patch, new_item, router, under_assignment};
pub use rules::{Condition, Node};
pub use service::{
    AssignmentFacts, ContentError, CourseSummary, FormView, ItemCreation, ItemDeletion, ItemRead,
    ItemRules, ItemUpdate, ModuleDeletion, NewItemInput, OpenError, announce_visible,
    authorize_edit_item, begin_create_item, begin_delete_item, begin_delete_module,
    begin_update_item, copy_course_content, course_summary, create_module, decorate_form,
    draft_items, editor_profiles, finish_create_item, finish_delete_item, finish_delete_module,
    finish_update_item, get_item, get_revision, get_tree, item_meta, lock_assignment_shared,
    lock_course_assignments, lock_facts, mark_done, may_read_item_files, next_deadlines,
    open_assignment, open_assignments, publish_item, publish_module, publish_revision,
    reorder_items, reorder_modules, replace_source_facts, search, sees_drafts, set_due_at,
    template_counts, unmark_done, update_module,
};
pub use state::ContentState;
pub use types::builtin_types;
pub use types::form::Holders;
pub use types::quiz::ResultQuestion;
pub use types::quiz::result_questions as quiz_result_questions;

// The ids of the built-in types whose own features other crates serve: the result of a quiz, the
// answers of a form, the text an agent writes (ADR-0097). Everything else asks the `Kind`.
pub mod form {
    pub use crate::types::form::ID;
}
pub mod quiz {
    pub use crate::types::quiz::ID;
}
pub mod text {
    pub use crate::types::text::ID;
}
// Seeds the references of an item for the tests of `learning::files`.
#[cfg(feature = "testing")]
pub use repo::replace_item_files;

#[cfg(test)]
mod tests;
