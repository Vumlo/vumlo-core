use std::collections::BTreeSet;
use std::fmt;

use api_types::{Feature, ProfileName};
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{Map, Value};
use sqlx::Postgres;
use sqlx::postgres::{PgTypeInfo, PgValueRef};
use sqlx::types::Decimal;
use uuid::Uuid;

use super::document::{DocumentErrors, EffectiveProfiles, ValidDocument, validate_document_within};
use crate::points;
use crate::policy::{CourseAccess, Release};
use crate::text_enum::text_enum;

const TITLE_MAX_LEN: usize = 200;

/// A value that failed validation. Each variant maps to one field of the API.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    #[error("must be 1 to {TITLE_MAX_LEN} characters")]
    Title,
    #[error("must be a number above 0 and at most 9999.99, with at most two decimals")]
    MaxPoints,
    #[error("unknown assignment type; available: text, no_submission")]
    AssignmentType,
    #[error("{0}")]
    Config(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentTitle(String);

impl ContentTitle {
    pub fn parse(raw: &str) -> Result<Self, ModelError> {
        let title = raw.trim();
        let len = title.chars().count();
        if (1..=TITLE_MAX_LEN).contains(&len) && !title.chars().any(char::is_control) {
            Ok(Self(title.to_owned()))
        } else {
            Err(ModelError::Title)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleStatus {
    Draft,
    Published,
}
text_enum!(ModuleStatus { Draft => "draft", Published => "published" });

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    Draft,
    Published,
}
text_enum!(ItemStatus { Draft => "draft", Published => "published" });

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Text,
    Assignment,
}
text_enum!(ItemKind { Text => "text", Assignment => "assignment" });

/// The most points one assignment can be worth (`numeric(6,2)`, ADR-0021).
const MAX_POINTS_LIMIT: Decimal = Decimal::from_parts(999_999, 0, 0, false, 2);

/// What an assignment is worth: above 0, at most 9999.99, at most two decimals. It knows nothing
/// of the grading scale (ADR-0022); the service applies that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaxPoints(Decimal);

impl MaxPoints {
    /// Accepts digits with an optional fraction of one or two digits: "20", "7.5", "20.00".
    pub fn parse(raw: &str) -> Result<Self, ModelError> {
        let value = points::parse_decimal(raw).ok_or(ModelError::MaxPoints)?;
        Self::from_decimal(value)
    }

    fn from_decimal(value: Decimal) -> Result<Self, ModelError> {
        if value > Decimal::ZERO && value <= MAX_POINTS_LIMIT && value.scale() <= 2 {
            Ok(Self(value))
        } else {
            Err(ModelError::MaxPoints)
        }
    }

    pub fn value(self) -> Decimal {
        self.0
    }
}

impl fmt::Display for MaxPoints {
    /// Always two decimals: "20.00".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.2}", self.0)
    }
}

impl sqlx::Type<Postgres> for MaxPoints {
    fn type_info() -> PgTypeInfo {
        <Decimal as sqlx::Type<Postgres>>::type_info()
    }
    fn compatible(ty: &PgTypeInfo) -> bool {
        <Decimal as sqlx::Type<Postgres>>::compatible(ty)
    }
}

impl<'r> sqlx::Decode<'r, Postgres> for MaxPoints {
    fn decode(value: PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let decimal = <Decimal as sqlx::Decode<Postgres>>::decode(value)?;
        Ok(Self::from_decimal(decimal)?)
    }
}

/// What an assignment type must provide (ADR-0008). It is the future plugin boundary, so it stays
/// small: auto-grading joins it with its own task.
pub trait AssignmentType {
    /// `Ok` when `config` is a valid configuration for this type.
    fn validate_config(&self, config: &Map<String, Value>) -> Result<(), ModelError>;

    /// Whether a student submits something for this type. Only such a type has the deadline
    /// settings of [`SubmissionSettings`] (ADR-0035).
    fn takes_submissions(&self) -> bool;

    /// Checks the `content` of a draft against the type, as narrowed by the institution's
    /// configuration (`profiles`) and by the assignment's own `config` (ADR-0025, ADR-0023). It
    /// is called only for a type that takes submissions.
    fn validate_submission(
        &self,
        config: &Map<String, Value>,
        content: &Value,
        profiles: &EffectiveProfiles,
    ) -> Result<ValidDocument, DocumentErrors>;
}

/// The types built into the server. The database column is plain text (ADR-0008).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinType {
    Text,
    /// Nothing is submitted: oral answers, class work, paper tests (ADR-0023). Graded by hand.
    NoSubmission,
}
text_enum!(BuiltinType { Text => "text", NoSubmission => "no_submission" });

impl BuiltinType {
    /// An unknown or not-yet-built type is an error that does not repeat the input.
    pub fn parse(raw: &str) -> Result<Self, ModelError> {
        raw.parse().map_err(|_| ModelError::AssignmentType)
    }
}

/// The features a `text` assignment's `config.editor.features` allows, in the order given, or
/// `None` when the config does not narrow the profile. Only `{"editor": {"features": [..]}}` is a
/// valid config; the messages name the position of a mistake and never repeat what was sent.
pub fn editor_features(config: &Map<String, Value>) -> Result<Option<Vec<Feature>>, ModelError> {
    let invalid = |message: &str| Err(ModelError::Config(message.to_owned()));
    if let Some(field) = config.keys().find(|key| *key != "editor") {
        return Err(ModelError::Config(format!(
            "unknown field `{field}`: the text type has only `editor`"
        )));
    }
    let Some(editor) = config.get("editor") else {
        return Ok(None);
    };
    let Some(editor) = editor.as_object() else {
        return invalid("`editor` must be an object");
    };
    if editor.keys().any(|key| key != "features") {
        return invalid("unknown field in `editor`: it has only `features`");
    }
    let Some(features) = editor.get("features") else {
        return Ok(None);
    };
    let Some(features) = features.as_array() else {
        return invalid("`editor.features` must be a list of feature names");
    };
    let mut chosen = Vec::with_capacity(features.len());
    for (index, value) in features.iter().enumerate() {
        let Some(feature) = value.as_str().and_then(Feature::from_name) else {
            return Err(ModelError::Config(format!(
                "`editor.features[{index}]` is not a feature (expected one of: {})",
                Feature::ALL
                    .iter()
                    .map(|feature| feature.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        };
        if chosen.contains(&feature) {
            return Err(ModelError::Config(format!(
                "`editor.features[{index}]` is listed twice"
            )));
        }
        chosen.push(feature);
    }
    Ok(Some(chosen))
}

impl AssignmentType for BuiltinType {
    fn validate_config(&self, config: &Map<String, Value>) -> Result<(), ModelError> {
        match self {
            // `text` can only narrow what the student's editor offers (ADR-0025).
            Self::Text => editor_features(config).map(|_| ()),
            // This type needs no settings; any field is a mistake.
            Self::NoSubmission => match config.keys().next() {
                None => Ok(()),
                Some(field) => Err(ModelError::Config(format!(
                    "unknown field `{field}`: the {} type has no settings",
                    self.as_str()
                ))),
            },
        }
    }

    fn takes_submissions(&self) -> bool {
        match self {
            Self::Text => true,
            Self::NoSubmission => false,
        }
    }

    fn validate_submission(
        &self,
        config: &Map<String, Value>,
        content: &Value,
        profiles: &EffectiveProfiles,
    ) -> Result<ValidDocument, DocumentErrors> {
        match self {
            Self::Text => {
                // A config that cannot be read narrows nothing further here: `validate_config`
                // refused it when it was written.
                let within: Option<BTreeSet<Feature>> = editor_features(config)
                    .ok()
                    .flatten()
                    .map(|features| features.into_iter().collect());
                validate_document_within(content, ProfileName::Student, profiles, within.as_ref())
            }
            Self::NoSubmission => Err(DocumentErrors::one("", "this type takes no submissions")),
        }
    }
}

/// What happens to a draft at `due_at` (ADR-0011). `AutoSubmit` is stored and validated and does
/// nothing until the background worker exists (ADR-0035).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnDeadline {
    KeepDraft,
    AutoSubmit,
}
text_enum!(OnDeadline { KeepDraft => "keep_draft", AutoSubmit => "auto_submit" });

/// The deadline settings of an assignment that takes submissions (ADR-0011, ADR-0035). Either
/// `due_at`, `on_deadline` and `accept_late` are all set or none is; `late_until` needs
/// `accept_late` and comes after `due_at`; `max_attempts` is at least 1. [`Self::resolve`] is
/// the one place that checks this, and the database repeats it in CHECK constraints.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SubmissionSettings {
    pub due_at: Option<DateTime<Utc>>,
    pub on_deadline: Option<OnDeadline>,
    pub accept_late: Option<bool>,
    /// `None` with `accept_late` set means late work is taken without an end.
    pub late_until: Option<DateTime<Utc>>,
    /// `None` means unlimited.
    pub max_attempts: Option<i32>,
}

/// A change to the settings, as the client sent it: for each field `None` leaves it, `Some(None)`
/// clears it and `Some(Some(_))` sets it. `on_deadline` and `max_attempts` stay raw so that a bad
/// value is reported on its field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SettingsPatch {
    pub due_at: Option<Option<DateTime<Utc>>>,
    pub on_deadline: Option<Option<String>>,
    pub accept_late: Option<Option<bool>>,
    pub late_until: Option<Option<DateTime<Utc>>>,
    pub max_attempts: Option<Option<i64>>,
}

/// One rule of the settings that the resulting state breaks, on one field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingProblem {
    pub field: &'static str,
    pub message: String,
}

impl SettingsPatch {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The first field the patch mentions, set or cleared. Call it on a patch that is not empty.
    pub fn first_field(&self) -> &'static str {
        [
            ("due_at", self.due_at.is_some()),
            ("on_deadline", self.on_deadline.is_some()),
            ("accept_late", self.accept_late.is_some()),
            ("late_until", self.late_until.is_some()),
            ("max_attempts", self.max_attempts.is_some()),
        ]
        .into_iter()
        .find_map(|(field, mentioned)| mentioned.then_some(field))
        .unwrap_or("settings")
    }

    /// The fields that set a value, for a type that takes no submissions to refuse. Clearing a
    /// field with `null` is not setting it.
    fn set_fields(&self) -> Vec<&'static str> {
        [
            ("due_at", matches!(self.due_at, Some(Some(_)))),
            ("on_deadline", matches!(self.on_deadline, Some(Some(_)))),
            ("accept_late", matches!(self.accept_late, Some(Some(_)))),
            ("late_until", matches!(self.late_until, Some(Some(_)))),
            ("max_attempts", matches!(self.max_attempts, Some(Some(_)))),
        ]
        .into_iter()
        .filter_map(|(field, set)| set.then_some(field))
        .collect()
    }
}

impl SubmissionSettings {
    /// The settings after `patch` is applied to `self`, with every broken rule reported at once.
    /// The rules are judged on the resulting state, so a field the patch leaves alone can be the
    /// one that makes it fail. `takes_submissions` is the type's answer: a type that takes none
    /// refuses any value.
    pub fn resolve(
        self,
        patch: &SettingsPatch,
        takes_submissions: bool,
    ) -> Result<Self, Vec<SettingProblem>> {
        if !takes_submissions {
            let problems: Vec<_> = patch
                .set_fields()
                .into_iter()
                .map(|field| SettingProblem {
                    field,
                    message: "this assignment type takes no submissions".to_owned(),
                })
                .collect();
            return if problems.is_empty() {
                Ok(self)
            } else {
                Err(problems)
            };
        }
        let mut problems = Vec::new();
        let mut problem = |field, message: &str| {
            problems.push(SettingProblem {
                field,
                message: message.to_owned(),
            });
        };
        let due_at = patch.due_at.unwrap_or(self.due_at);
        let accept_late = patch.accept_late.unwrap_or(self.accept_late);
        let late_until = patch.late_until.unwrap_or(self.late_until);
        let on_deadline = match &patch.on_deadline {
            None => self.on_deadline,
            Some(None) => None,
            Some(Some(raw)) => match raw.parse::<OnDeadline>() {
                Ok(value) => Some(value),
                Err(_) => {
                    problem("on_deadline", "must be keep_draft or auto_submit");
                    self.on_deadline
                }
            },
        };
        let max_attempts = match patch.max_attempts {
            None => self.max_attempts,
            Some(None) => None,
            Some(Some(raw)) => match i32::try_from(raw).ok().filter(|n| *n >= 1) {
                Some(value) => Some(value),
                None => {
                    problem("max_attempts", "must be a whole number of at least 1");
                    self.max_attempts
                }
            },
        };
        let bad_on_deadline =
            matches!(&patch.on_deadline, Some(Some(raw)) if raw.parse::<OnDeadline>().is_err());
        match due_at {
            Some(_) => {
                if on_deadline.is_none() && !bad_on_deadline {
                    problem("on_deadline", "is required when due_at is set");
                }
                if accept_late.is_none() {
                    problem("accept_late", "is required when due_at is set");
                }
            }
            None => {
                if on_deadline.is_some() {
                    problem("on_deadline", "needs due_at");
                }
                if accept_late.is_some() {
                    problem("accept_late", "needs due_at");
                }
            }
        }
        if let Some(late_until) = late_until {
            if accept_late != Some(true) {
                problem("late_until", "needs accept_late to be true");
            } else if due_at.is_some_and(|due_at| late_until <= due_at) {
                problem("late_until", "must be after due_at");
            }
        }
        if problems.is_empty() {
            Ok(Self {
                due_at,
                on_deadline,
                accept_late,
                late_until,
                max_attempts,
            })
        } else {
            Err(problems)
        }
    }
}

/// The assignment part of an item of kind `assignment`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub assignment_type: BuiltinType,
    pub config: Map<String, Value>,
    pub max_points: MaxPoints,
    pub is_bonus: bool,
    pub settings: SubmissionSettings,
    /// The day an in-class (`no_submission`) assignment takes place (ADR-0049).
    pub class_on: Option<NaiveDate>,
}

/// What the course tree shows of an assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignmentSummary {
    pub assignment_type: BuiltinType,
    pub max_points: MaxPoints,
    pub is_bonus: bool,
    pub settings: SubmissionSettings,
}

/// An assignment that a user may view, with how they relate to its course (see
/// `content::open_assignment`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenedAssignment {
    pub course_id: Uuid,
    pub assignment_type: BuiltinType,
    pub access: CourseAccess,
}

/// A top-level module (`parent_id` is `None`) or a sub-module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub id: Uuid,
    pub course_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub position: i32,
    pub title: String,
    pub status: ModuleStatus,
    pub publish_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Module {
    pub fn release(&self) -> Release {
        Release {
            published: self.status == ModuleStatus::Published,
            publish_at: self.publish_at,
        }
    }
}

/// A `text` item or an assignment. `body` is a block document (ADR-0025) as stored; for an
/// assignment it is the description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: Uuid,
    pub module_id: Uuid,
    pub course_id: Uuid,
    pub position: i32,
    pub kind: ItemKind,
    pub title: String,
    pub body: Map<String, Value>,
    pub status: ItemStatus,
    pub publish_at: Option<DateTime<Utc>>,
    /// Present exactly for items of kind `assignment`.
    pub assignment: Option<Assignment>,
    /// What students read; `None` until the item is published first (ADR-0049). `title` and `body`
    /// above are the working copy that staff edit.
    pub published: Option<PublishedSlot>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The title and body of an item as its students read them (ADR-0049).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedSlot {
    pub title: String,
    pub body: Map<String, Value>,
    pub at: DateTime<Utc>,
}

impl Item {
    pub fn release(&self) -> Release {
        Release {
            published: self.status == ItemStatus::Published,
            publish_at: self.publish_at,
        }
    }

    /// The working copy differs from what students read. False for an item never published.
    pub fn has_unpublished_changes(&self) -> bool {
        self.published
            .as_ref()
            .is_some_and(|slot| slot.title != self.title || slot.body != self.body)
    }

    /// The item as a student sees it: the published title and body in place of the working ones.
    pub fn as_seen_by_students(mut self) -> Self {
        if let Some(slot) = self.published.take() {
            self.title = slot.title;
            self.body = slot.body;
        }
        self
    }
}

/// An item as the course tree lists it: no body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemSummary {
    pub id: Uuid,
    pub kind: ItemKind,
    pub title: String,
    pub status: ItemStatus,
    pub publish_at: Option<DateTime<Utc>>,
    pub position: i32,
    pub assignment: Option<AssignmentSummary>,
}

impl ItemSummary {
    pub fn release(&self) -> Release {
        Release {
            published: self.status == ItemStatus::Published,
            publish_at: self.publish_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentSubModule {
    pub module: Module,
    pub items: Vec<ItemSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentModule {
    pub module: Module,
    pub submodules: Vec<ContentSubModule>,
    pub items: Vec<ItemSummary>,
}

/// A change to a module. `publish_at`: `None` leaves it, `Some(None)` clears it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModulePatch {
    pub title: Option<ContentTitle>,
    pub publish_at: Option<Option<DateTime<Utc>>>,
}

impl ModulePatch {
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.publish_at.is_none()
    }
}

/// A change to an item. `body` is the raw JSON the client sent; it replaces the whole body when
/// set, after the service has authorized the change and validated it as a document. `config`,
/// `max_points` and `is_bonus` apply to assignments only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemPatch {
    pub title: Option<ContentTitle>,
    pub body: Option<Value>,
    pub publish_at: Option<Option<DateTime<Utc>>>,
    pub config: Option<Map<String, Value>>,
    pub max_points: Option<MaxPoints>,
    pub is_bonus: Option<bool>,
    pub settings: SettingsPatch,
    /// `Some(None)` clears the class day. Only an in-class assignment has one.
    pub class_on: Option<Option<NaiveDate>>,
}

impl ItemPatch {
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.body.is_none()
            && self.publish_at.is_none()
            && !self.touches_assignment()
    }

    /// True when the patch sets a field that lives on the assignment row.
    pub fn touches_assignment(&self) -> bool {
        self.config.is_some()
            || self.max_points.is_some()
            || self.is_bonus.is_some()
            || self.class_on.is_some()
            || !self.settings.is_empty()
    }
}

/// What the editor needs to know about an item's unpublished changes (ADR-0049).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    pub has_unpublished_changes: bool,
    pub last_published_at: Option<DateTime<Utc>>,
    /// The size of the working body as JSON.
    pub size_bytes: i64,
}

impl Revision {
    pub fn of(item: &Item) -> Self {
        Self {
            has_unpublished_changes: item.has_unpublished_changes(),
            last_published_at: item.published.as_ref().map(|slot| slot.at),
            size_bytes: serde_json::to_vec(&item.body)
                .map_or(0, |bytes| i64::try_from(bytes.len()).unwrap_or(i64::MAX)),
        }
    }
}

/// Where a student stands on an assignment (ADR-0049, point 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkStatus {
    NotStarted,
    Draft,
    Submitted,
    Graded,
    /// The window closed with nothing submitted.
    Missed,
    /// In-class work: nothing to submit.
    InClass,
}
text_enum!(WorkStatus {
    NotStarted => "not_started",
    Draft => "draft",
    Submitted => "submitted",
    Graded => "graded",
    Missed => "missed",
    InClass => "in_class",
});

impl WorkStatus {
    /// The one rule: a published value wins, then in-class work, then an attempt, then a draft, then
    /// a closed window, then nothing yet.
    pub fn of(
        graded: bool,
        takes_submissions: bool,
        attempts: i64,
        has_draft: bool,
        closed: bool,
    ) -> Self {
        if graded {
            Self::Graded
        } else if !takes_submissions {
            Self::InClass
        } else if attempts > 0 {
            Self::Submitted
        } else if has_draft {
            Self::Draft
        } else if closed {
            Self::Missed
        } else {
            Self::NotStarted
        }
    }
}

/// A student's own standing on one assignment. Only published values appear here (AGENTS section 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkFacts {
    pub assignment_id: Uuid,
    pub course_id: Uuid,
    pub status: WorkStatus,
    /// The published score; `None` before grades are published.
    pub score: Option<Decimal>,
    pub published_at: Option<DateTime<Utc>>,
    pub draft_saved_at: Option<DateTime<Utc>>,
    pub attempts_used: i64,
}

/// What a teacher still has to do on one assignment, over the current students.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaffFacts {
    pub assignment_id: Uuid,
    pub submitted: i64,
    pub to_grade: i64,
    pub draft_grades: i64,
    pub graded_total: i64,
}

/// The facts of one item that the tree does not carry (ADR-0049, point 5).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemFacts {
    pub read_minutes: Option<i64>,
    pub class_on: Option<NaiveDate>,
    /// Staff: published, with changes students do not see yet.
    pub edited: Option<bool>,
    /// A student's own standing (students only).
    pub work: Option<WorkFacts>,
    /// The counts for teachers (staff only).
    pub staff: Option<StaffFacts>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_of_work_follows_one_order_of_rules() {
        use WorkStatus::*;
        // (graded, takes_submissions, attempts, draft, closed)
        let cases = [
            ((true, true, 2, true, true), Graded),
            ((true, false, 0, false, false), Graded),
            ((false, false, 0, false, true), InClass),
            ((false, true, 1, true, true), Submitted),
            ((false, true, 0, true, true), Draft),
            ((false, true, 0, false, true), Missed),
            ((false, true, 0, false, false), NotStarted),
        ];
        for ((graded, takes, attempts, draft, closed), expected) in cases {
            assert_eq!(
                WorkStatus::of(graded, takes, attempts, draft, closed),
                expected,
                "{graded} {takes} {attempts} {draft} {closed}"
            );
        }
    }

    #[test]
    fn a_revision_knows_what_changed_since_the_item_was_published() {
        let mut item = Item {
            id: Uuid::nil(),
            module_id: Uuid::nil(),
            course_id: Uuid::nil(),
            position: 1,
            kind: ItemKind::Text,
            title: "Lecture".to_owned(),
            body: Map::new(),
            status: ItemStatus::Draft,
            publish_at: None,
            assignment: None,
            published: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(
            !item.has_unpublished_changes(),
            "a draft has nothing to compare with"
        );
        item.published = Some(PublishedSlot {
            title: "Lecture".to_owned(),
            body: Map::new(),
            at: Utc::now(),
        });
        assert!(!item.has_unpublished_changes());
        item.title = "Lecture, revised".to_owned();
        assert!(item.has_unpublished_changes());
        let seen = item.clone().as_seen_by_students();
        assert_eq!(seen.title, "Lecture");
        assert!(seen.published.is_none());
        assert!(Revision::of(&item).has_unpublished_changes);
    }

    #[test]
    fn title_is_trimmed_and_bounded() {
        assert_eq!(
            ContentTitle::parse("  Lectures ").unwrap().as_str(),
            "Lectures"
        );
        assert!(ContentTitle::parse("").is_err());
        assert!(ContentTitle::parse("   ").is_err());
        assert!(ContentTitle::parse("a\nb").is_err());
        assert!(ContentTitle::parse("a\u{7f}b").is_err());
        assert!(ContentTitle::parse(&"ł".repeat(200)).is_ok());
        assert!(ContentTitle::parse(&"ł".repeat(201)).is_err());
    }

    #[test]
    fn enums_round_trip() {
        for status in [ModuleStatus::Draft, ModuleStatus::Published] {
            assert_eq!(status.as_str().parse::<ModuleStatus>(), Ok(status));
        }
        for status in [ItemStatus::Draft, ItemStatus::Published] {
            assert_eq!(status.as_str().parse::<ItemStatus>(), Ok(status));
        }
        assert_eq!("text".parse::<ItemKind>(), Ok(ItemKind::Text));
        assert!("lesson".parse::<ItemKind>().is_err());
        assert_eq!("assignment".parse::<ItemKind>(), Ok(ItemKind::Assignment));
        assert!("quiz".parse::<ItemKind>().is_err());
    }

    #[test]
    fn max_points_accepts_decimal_strings_up_to_two_decimals() {
        for ok in ["20", "7.5", "20.00", "0.01", "9999.99", "9999", "0012"] {
            assert!(MaxPoints::parse(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn max_points_rejects_everything_else() {
        for bad in [
            "0",
            "0.00",
            "-1",
            "10000",
            "9999.991",
            "10000.00",
            "1.005",
            "abc",
            "",
            ".5",
            "5.",
            "+5",
            "1e2",
            " 5",
            "5 ",
            "1,5",
            "1.2.3",
            "99999999999999999999",
        ] {
            assert_eq!(MaxPoints::parse(bad), Err(ModelError::MaxPoints), "{bad:?}");
        }
    }

    #[test]
    fn max_points_display_has_exactly_two_decimals() {
        for (raw, shown) in [
            ("20", "20.00"),
            ("7.5", "7.50"),
            ("20.00", "20.00"),
            ("0.01", "0.01"),
            ("9999.99", "9999.99"),
        ] {
            assert_eq!(MaxPoints::parse(raw).unwrap().to_string(), shown);
        }
    }

    #[test]
    fn the_type_enum_parses_text_and_no_submission_and_nothing_else() {
        assert_eq!(BuiltinType::parse("text"), Ok(BuiltinType::Text));
        assert_eq!(BuiltinType::Text.as_str(), "text");
        assert_eq!(
            BuiltinType::parse("no_submission"),
            Ok(BuiltinType::NoSubmission)
        );
        assert_eq!(BuiltinType::NoSubmission.as_str(), "no_submission");
        for bad in ["form", "", "Text", "No_Submission", "no-submission"] {
            assert_eq!(
                BuiltinType::parse(bad),
                Err(ModelError::AssignmentType),
                "{bad:?}"
            );
        }
        assert!(!ModelError::AssignmentType.to_string().contains("form"));
    }

    #[test]
    fn the_text_and_no_submission_types_take_an_empty_config_only() {
        for assignment_type in [BuiltinType::Text, BuiltinType::NoSubmission] {
            let mut config = Map::new();
            assert_eq!(assignment_type.validate_config(&config), Ok(()));
            config.insert("limit".to_owned(), Value::from(5));
            let error = assignment_type.validate_config(&config).unwrap_err();
            let message = error.to_string();
            assert!(message.contains("limit"), "{message}");
            assert!(message.contains(assignment_type.as_str()), "{message}");
        }
    }

    fn at(day: u32) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(&format!("2030-01-{day:02}T12:00:00Z"))
            .unwrap()
            .with_timezone(&Utc)
    }

    fn deadline_patch() -> SettingsPatch {
        SettingsPatch {
            due_at: Some(Some(at(10))),
            on_deadline: Some(Some("keep_draft".to_owned())),
            accept_late: Some(Some(false)),
            ..SettingsPatch::default()
        }
    }

    fn fields(result: Result<SubmissionSettings, Vec<SettingProblem>>) -> Vec<&'static str> {
        result
            .expect_err("the settings must be refused")
            .into_iter()
            .map(|problem| problem.field)
            .collect()
    }

    #[test]
    fn a_deadline_comes_with_what_happens_at_it_and_whether_late_work_is_taken() {
        let set = SubmissionSettings::default()
            .resolve(&deadline_patch(), true)
            .unwrap();
        assert_eq!(set.due_at, Some(at(10)));
        assert_eq!(set.on_deadline, Some(OnDeadline::KeepDraft));
        assert_eq!(set.accept_late, Some(false));
        assert_eq!((set.late_until, set.max_attempts), (None, None));

        for (what, patch, expected) in [
            (
                "due_at alone",
                SettingsPatch {
                    due_at: Some(Some(at(10))),
                    ..SettingsPatch::default()
                },
                vec!["on_deadline", "accept_late"],
            ),
            (
                "on_deadline alone",
                SettingsPatch {
                    on_deadline: Some(Some("auto_submit".to_owned())),
                    ..SettingsPatch::default()
                },
                vec!["on_deadline"],
            ),
            (
                "accept_late alone",
                SettingsPatch {
                    accept_late: Some(Some(true)),
                    ..SettingsPatch::default()
                },
                vec!["accept_late"],
            ),
        ] {
            let result = SubmissionSettings::default().resolve(&patch, true);
            assert_eq!(fields(result), expected, "{what}");
        }
    }

    #[test]
    fn late_until_needs_accept_late_and_a_later_moment_than_due_at() {
        let with_late = |accept: bool, late_until: DateTime<Utc>| SettingsPatch {
            accept_late: Some(Some(accept)),
            late_until: Some(Some(late_until)),
            ..deadline_patch()
        };
        let ok = SubmissionSettings::default().resolve(&with_late(true, at(11)), true);
        assert_eq!(ok.unwrap().late_until, Some(at(11)));
        for (what, patch) in [
            ("accept_late false", with_late(false, at(11))),
            ("equal to due_at", with_late(true, at(10))),
            ("before due_at", with_late(true, at(9))),
        ] {
            let result = SubmissionSettings::default().resolve(&patch, true);
            assert_eq!(fields(result), ["late_until"], "{what}");
        }
        let alone = SettingsPatch {
            late_until: Some(Some(at(11))),
            ..SettingsPatch::default()
        };
        assert_eq!(
            fields(SubmissionSettings::default().resolve(&alone, true)),
            ["late_until"]
        );
    }

    #[test]
    fn max_attempts_is_a_whole_number_of_at_least_one() {
        let attempts = |value: i64| SettingsPatch {
            max_attempts: Some(Some(value)),
            ..SettingsPatch::default()
        };
        for good in [1, 2, i64::from(i32::MAX)] {
            let set = SubmissionSettings::default().resolve(&attempts(good), true);
            assert_eq!(
                set.unwrap().max_attempts,
                i32::try_from(good).ok(),
                "{good}"
            );
        }
        for bad in [0, -1, i64::from(i32::MAX) + 1] {
            let result = SubmissionSettings::default().resolve(&attempts(bad), true);
            assert_eq!(fields(result), ["max_attempts"], "{bad}");
        }
    }

    #[test]
    fn a_bad_on_deadline_is_one_problem_on_its_field() {
        let patch = SettingsPatch {
            on_deadline: Some(Some("later".to_owned())),
            ..deadline_patch()
        };
        assert_eq!(
            fields(SubmissionSettings::default().resolve(&patch, true)),
            ["on_deadline"]
        );
    }

    #[test]
    fn the_rules_are_judged_on_the_resulting_state() {
        let full = SubmissionSettings::default()
            .resolve(&deadline_patch(), true)
            .unwrap();
        // Absent fields change nothing.
        assert_eq!(full.resolve(&SettingsPatch::default(), true).unwrap(), full);
        // Clearing only the deadline leaves the other two without it.
        let only_due = SettingsPatch {
            due_at: Some(None),
            ..SettingsPatch::default()
        };
        assert_eq!(
            fields(full.resolve(&only_due, true)),
            ["on_deadline", "accept_late"]
        );
        // Clearing the three together removes the deadline.
        let all = SettingsPatch {
            due_at: Some(None),
            on_deadline: Some(None),
            accept_late: Some(None),
            ..SettingsPatch::default()
        };
        assert_eq!(
            full.resolve(&all, true).unwrap(),
            SubmissionSettings::default()
        );
        // A change of accept_late alone breaks an existing late_until.
        let late = full
            .resolve(
                &SettingsPatch {
                    accept_late: Some(Some(true)),
                    late_until: Some(Some(at(12))),
                    ..SettingsPatch::default()
                },
                true,
            )
            .unwrap();
        let strict = SettingsPatch {
            accept_late: Some(Some(false)),
            ..SettingsPatch::default()
        };
        assert_eq!(fields(late.resolve(&strict, true)), ["late_until"]);
        // Moving due_at past an existing late_until does too.
        let later = SettingsPatch {
            due_at: Some(Some(at(20))),
            ..SettingsPatch::default()
        };
        assert_eq!(fields(late.resolve(&later, true)), ["late_until"]);
        // max_attempts clears with null and is independent of the deadline.
        let two = full
            .resolve(
                &SettingsPatch {
                    max_attempts: Some(Some(2)),
                    ..SettingsPatch::default()
                },
                true,
            )
            .unwrap();
        let unlimited = SettingsPatch {
            max_attempts: Some(None),
            ..SettingsPatch::default()
        };
        assert_eq!(two.resolve(&unlimited, true).unwrap().max_attempts, None);
    }

    #[test]
    fn a_type_that_takes_no_submissions_refuses_every_value_but_accepts_null() {
        let everything = SettingsPatch {
            max_attempts: Some(Some(1)),
            ..deadline_patch()
        };
        assert_eq!(
            fields(SubmissionSettings::default().resolve(&everything, false)),
            ["due_at", "on_deadline", "accept_late", "max_attempts"]
        );
        let nulls = SettingsPatch {
            due_at: Some(None),
            on_deadline: Some(None),
            accept_late: Some(None),
            late_until: Some(None),
            max_attempts: Some(None),
        };
        assert_eq!(
            SubmissionSettings::default().resolve(&nulls, false),
            Ok(SubmissionSettings::default())
        );
        assert!(BuiltinType::Text.takes_submissions());
        assert!(!BuiltinType::NoSubmission.takes_submissions());
    }

    fn features(config: Value) -> Result<Option<Vec<Feature>>, ModelError> {
        editor_features(config.as_object().unwrap())
    }

    #[test]
    fn the_editor_features_of_a_text_config_are_a_list_of_known_unique_names() {
        assert_eq!(features(serde_json::json!({})), Ok(None));
        assert_eq!(features(serde_json::json!({"editor": {}})), Ok(None));
        assert_eq!(
            features(serde_json::json!({"editor": {"features": ["code", "headings"]}})),
            Ok(Some(vec![Feature::Code, Feature::Headings]))
        );
        assert_eq!(
            features(serde_json::json!({"editor": {"features": []}})),
            Ok(Some(vec![]))
        );
        for (what, config) in [
            ("another key", serde_json::json!({"limit": 1})),
            ("editor not an object", serde_json::json!({"editor": []})),
            (
                "another editor key",
                serde_json::json!({"editor": {"a": 1}}),
            ),
            (
                "features not a list",
                serde_json::json!({"editor": {"features": "code"}}),
            ),
            (
                "unknown feature",
                serde_json::json!({"editor": {"features": ["code", "blink"]}}),
            ),
            (
                "not a string",
                serde_json::json!({"editor": {"features": [1]}}),
            ),
            (
                "twice",
                serde_json::json!({"editor": {"features": ["code", "code"]}}),
            ),
        ] {
            let error = features(config).expect_err(what);
            assert!(!error.to_string().contains("blink"), "{what}: {error}");
        }
        let message = features(serde_json::json!({"editor": {"features": ["code", "blink"]}}))
            .unwrap_err()
            .to_string();
        assert!(message.contains("features[1]"), "{message}");
        // The type validates through the same function.
        let config = serde_json::json!({"editor": {"features": ["blink"]}});
        assert!(
            BuiltinType::Text
                .validate_config(config.as_object().unwrap())
                .is_err()
        );
        let config = serde_json::json!({"editor": {"features": ["code"]}});
        assert!(
            BuiltinType::Text
                .validate_config(config.as_object().unwrap())
                .is_ok()
        );
        assert!(
            BuiltinType::NoSubmission
                .validate_config(config.as_object().unwrap())
                .is_err()
        );
    }

    #[test]
    fn empty_patches_are_recognised() {
        assert!(ModulePatch::default().is_empty());
        assert!(ItemPatch::default().is_empty());
        let sets_points = ItemPatch {
            max_points: Some(MaxPoints::parse("5").unwrap()),
            ..ItemPatch::default()
        };
        assert!(!sets_points.is_empty());
        assert!(sets_points.touches_assignment());
        let clears = ModulePatch {
            publish_at: Some(None),
            ..ModulePatch::default()
        };
        assert!(!clears.is_empty());
    }
}
