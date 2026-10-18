//! Unpublished revisions and per-item facts (ADR-0049). Points are decimal strings with two
//! decimals (ADR-0021).

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Where a student stands on an assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkStatus {
    NotStarted,
    Draft,
    Submitted,
    /// A grade is published.
    Graded,
    /// The window closed with nothing submitted.
    Missed,
    /// In-class work: nothing to submit.
    InClass,
}

/// What the editor shows about the unpublished changes of an item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ItemRevision {
    /// The working title or body differs from what students read.
    pub has_unpublished_changes: bool,
    /// When students last got a new version; `null` for an item never published.
    pub last_published_at: Option<DateTime<Utc>>,
    /// The size of the working body as JSON.
    pub size_bytes: i64,
}

/// The facts of one item that the course tree does not carry. Only the facts that apply to the
/// item and to the caller are present.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ItemMeta {
    /// Lectures: minutes to read the body the caller sees, at least 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_minutes: Option<u32>,
    /// In-class assignments: the day they take place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_on: Option<NaiveDate>,
    /// Staff: the item is published and has changes students do not see yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited: Option<bool>,
    /// Students, assignments: the caller's own standing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<WorkStatus>,
    /// Students: the published score; absent until grades are published.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_saved_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempts_used: Option<u32>,
    /// Staff, assignments: current students who submitted at least once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted: Option<u32>,
    /// Staff: of those, the ones without a score yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_grade: Option<u32>,
    /// Staff: grades whose draft differs from what is published.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_grades: Option<u32>,
    /// Staff: current students of the course.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graded_total: Option<u32>,
}

/// The facts by item id, for the items the caller may see.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ItemMetaMap {
    pub items: HashMap<String, ItemMeta>,
}
