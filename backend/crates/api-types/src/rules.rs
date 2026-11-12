//! Completion and access rules (ADR-0092). A module or an item can be locked until the student has
//! done something. Locked is not hidden: the node stays in the course tree with its title, and its
//! text, files and questions, and the right to submit, wait for the rule.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConditionKind {
    /// An item is done, or every item in a module is.
    Completed,
    /// The published score of an assignment is at least `points`.
    ScoreAtLeast,
}

/// One condition of a rule.
///
/// `completed` names an item (`item_id`) or a module (`module_id`), not both. An assignment is done
/// when the student has submitted it or has a published score; a text or file item, when the
/// student marks it (`PUT /items/{id}/completion`); a module, when it has items the student can see
/// and all of them are done.
///
/// `score_at_least` names an assignment (`item_id`) and `points`, a decimal string above 0, at most
/// what the assignment is worth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Condition {
    pub kind: ConditionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<String>,
}

/// What locks a module or an item: every condition must hold. A rule on a module also locks the
/// items and sub-modules in it. At most 10 conditions; none removes the rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AccessRule {
    pub all: Vec<Condition>,
}

/// A condition that does not hold yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct UnmetCondition {
    pub condition: Condition,
    /// The title of what the condition names, when the student can see it; `null` for something
    /// they cannot, so that a draft is not given away.
    pub title: Option<String>,
}

/// Where a student stands on a module or an item. Students only; staff are never locked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Standing {
    /// Finished: marked by the student, submitted or graded, or every item of the module is.
    pub done: bool,
    /// What still keeps it closed. Empty when it is open. A locked item reads as its title only:
    /// an empty `body` and no questions.
    pub locked: Vec<UnmetCondition>,
}
