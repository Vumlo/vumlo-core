//! How points become a result (ADR-0089): groups of assignments with a cap, a "best K" rule,
//! practice and bonus, the course total against a target, and the setup status a teacher sees.
//! Pure functions over values: no SQL, no actor, no clock.
//!
//! A course shows its groups in one of two layouts. In `course` a group stands for a top-level
//! module; in `custom` the groups are the teacher's own. The caller resolves which group each
//! assignment is in; this module does not care where the answer came from.

use sqlx::types::Decimal;
use uuid::Uuid;

use common::text_enum;

/// Where an assignment's group comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    /// A group stands for a top-level module of the course.
    #[default]
    Course,
    /// The teacher's own groups.
    Custom,
}
text_enum!(Layout { Course => "course", Custom => "custom" });

/// What a course counts toward: the points that make a full result and the mark that passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub target: Decimal,
    pub pass_mark: Option<Decimal>,
    pub layout: Layout,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            target: Decimal::from(100),
            pass_mark: None,
            layout: Layout::Course,
        }
    }
}

/// A group and its rules. `key` is the top-level module's id in the `course` layout and the
/// group's own id in the `custom` one; `row` is the `gradebook.groups` row when there is one (a
/// module with no rule set has none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupDef {
    pub key: Uuid,
    pub row: Option<Uuid>,
    pub title: String,
    pub cap: Option<Decimal>,
    pub best_k: Option<u32>,
    /// False for practice: shown and graded, not part of the total.
    pub counts: bool,
    /// True for extra: adds to the total, not to what a student can still reach.
    pub bonus: bool,
}

impl GroupDef {
    /// A group with no rules: everything counts.
    pub fn plain(key: Uuid, title: impl Into<String>) -> Self {
        Self {
            key,
            row: None,
            title: title.into(),
            cap: None,
            best_k: None,
            counts: true,
            bonus: false,
        }
    }
}

/// One assignment as the computation sees it. `score` is the value being summed: the published
/// one for a student, the published or the draft one for staff; `None` counts as nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub assignment_id: Uuid,
    pub max_points: Decimal,
    pub is_bonus: bool,
    /// The `key` of its group; `None` when it is in no group ("Not sorted" or loose).
    pub group: Option<Uuid>,
    pub score: Option<Decimal>,
}

/// What one group adds up to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupResult {
    /// `None` for the assignments that are in no group.
    pub key: Option<Uuid>,
    pub total: Decimal,
    /// What a student can reach in the group, bonus left out.
    pub possible: Decimal,
    /// The cap cut the sum.
    pub capped: bool,
    /// The assignments whose marks are part of `total`, in the order they were given.
    pub counted: Vec<Uuid>,
    /// Left out by the "best K" rule.
    pub dropped: Vec<Uuid>,
    /// Every assignment of the group, in the order they were given.
    pub assignments: Vec<Uuid>,
}

/// The result of a student, or of a draft of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The total, never more than the target.
    pub total: Decimal,
    /// The sum before the target cut it.
    pub raw_total: Decimal,
    pub possible: Decimal,
    /// `None` when the course has no pass mark.
    pub passed: Option<bool>,
    /// Groups in the order of `groups`, then the loose assignments (key `None`) last. A group
    /// with no assignment is left out.
    pub groups: Vec<GroupResult>,
}

fn sum(values: impl IntoIterator<Item = Decimal>) -> Decimal {
    values.into_iter().fold(Decimal::ZERO, |all, one| all + one)
}

/// The `k` largest values, summed.
fn largest(mut values: Vec<Decimal>, k: usize) -> Decimal {
    values.sort_by(|a, b| b.cmp(a));
    sum(values.into_iter().take(k))
}

fn result_of(def: Option<&GroupDef>, members: &[&Entry]) -> GroupResult {
    let assignments: Vec<Uuid> = members.iter().map(|e| e.assignment_id).collect();
    let Some(def) = def else {
        // Loose assignments: everything counts, there is no cap.
        return GroupResult {
            key: None,
            total: sum(members.iter().map(|e| e.score.unwrap_or_default())),
            possible: sum(members.iter().filter(|e| !e.is_bonus).map(|e| e.max_points)),
            capped: false,
            counted: assignments.clone(),
            dropped: Vec::new(),
            assignments,
        };
    };
    if !def.counts {
        return GroupResult {
            key: Some(def.key),
            total: Decimal::ZERO,
            possible: Decimal::ZERO,
            capped: false,
            counted: Vec::new(),
            dropped: Vec::new(),
            assignments,
        };
    }
    // The K best marks, ties by the order given; an empty mark is a zero.
    let k = def
        .best_k
        .map_or(members.len(), |k| (k as usize).min(members.len()));
    let mut order: Vec<usize> = (0..members.len()).collect();
    order.sort_by(|&a, &b| {
        let score = |i: usize| members[i].score.unwrap_or_default();
        score(b).cmp(&score(a)).then(a.cmp(&b))
    });
    let mut chosen: Vec<usize> = order.iter().copied().take(k).collect();
    chosen.sort_unstable();
    let counted: Vec<Uuid> = chosen.iter().map(|&i| members[i].assignment_id).collect();
    let dropped: Vec<Uuid> = (0..members.len())
        .filter(|i| !chosen.contains(i))
        .map(|i| members[i].assignment_id)
        .collect();
    let raw = sum(chosen.iter().map(|&i| members[i].score.unwrap_or_default()));
    let (total, capped) = match def.cap {
        Some(cap) if raw > cap => (cap, true),
        _ => (raw, false),
    };
    let possible = if def.bonus {
        Decimal::ZERO
    } else {
        let regular: Vec<Decimal> = members
            .iter()
            .filter(|e| !e.is_bonus)
            .map(|e| e.max_points)
            .collect();
        let base = match def.best_k {
            Some(k) => largest(regular, k as usize),
            None => sum(regular),
        };
        def.cap.map_or(base, |cap| base.min(cap))
    };
    GroupResult {
        key: Some(def.key),
        total,
        possible,
        capped,
        counted,
        dropped,
        assignments,
    }
}

/// Adds the assignments up. `entries` are in the order they are shown; `groups` are in the order
/// their groups are shown. An entry whose group is not in `groups` is loose.
pub fn compute(settings: &Settings, groups: &[GroupDef], entries: &[Entry]) -> Outcome {
    let mut results = Vec::new();
    for def in groups {
        let members: Vec<&Entry> = entries
            .iter()
            .filter(|e| e.group == Some(def.key))
            .collect();
        if !members.is_empty() {
            results.push(result_of(Some(def), &members));
        }
    }
    let loose: Vec<&Entry> = entries
        .iter()
        .filter(|e| !e.group.is_some_and(|key| groups.iter().any(|g| g.key == key)))
        .collect();
    if !loose.is_empty() {
        results.push(result_of(None, &loose));
    }
    let raw_total = sum(results.iter().map(|g| g.total));
    let total = raw_total.min(settings.target);
    Outcome {
        total,
        raw_total,
        possible: sum(results.iter().map(|g| g.possible)),
        passed: settings.pass_mark.map(|mark| total >= mark),
        groups: results,
    }
}

/// What is left to do before the gradebook is in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    /// Assignments in no group, in the `custom` layout ("Not sorted").
    pub unsorted: u32,
    /// Groups with no assignment in them.
    pub empty_groups: Vec<Uuid>,
    pub possible: Decimal,
    pub target: Decimal,
    /// What a student can reach without bonus is at least the target.
    pub reaches_target: bool,
}

/// The status of a gradebook, from the course's own numbers (`entries` with no scores).
pub fn setup(settings: &Settings, groups: &[GroupDef], entries: &[Entry]) -> Setup {
    let outcome = compute(settings, groups, entries);
    let unsorted = if settings.layout == Layout::Custom {
        entries
            .iter()
            .filter(|e| !e.group.is_some_and(|key| groups.iter().any(|g| g.key == key)))
            .count() as u32
    } else {
        0
    };
    let empty_groups = groups
        .iter()
        .filter(|g| g.row.is_some() && !entries.iter().any(|e| e.group == Some(g.key)))
        .filter_map(|g| g.row)
        .collect();
    Setup {
        unsorted,
        empty_groups,
        reaches_target: outcome.possible >= settings.target,
        possible: outcome.possible,
        target: settings.target,
    }
}

/// A proposed group: assignments whose titles share a stem ("Lab 1", "Lab 2" → "Lab").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub title: String,
    pub assignments: Vec<Uuid>,
}

/// The stem of a title: what comes before a colon, up to the first digit ("Lab 3 - SVD" and
/// "Lab 1: Matrices" are both "Lab").
fn stem(title: &str) -> String {
    let head = title.split(':').next().unwrap_or(title);
    let head = head.split(|c: char| c.is_ascii_digit()).next().unwrap_or(head);
    head.trim_end_matches(|c: char| c.is_whitespace() || matches!(c, '.' | '-' | '_' | '/' | '#' | '№'))
        .trim()
        .to_owned()
}

/// Groups of at least two assignments whose titles share a stem, in the order the first of each
/// appears. Only a proposal: the teacher confirms it.
pub fn suggest(items: &[(Uuid, String)]) -> Vec<Suggestion> {
    let mut found: Vec<(String, Suggestion)> = Vec::new();
    for (id, title) in items {
        let stem = stem(title);
        if stem.chars().count() < 2 {
            continue;
        }
        let key = stem.to_lowercase();
        match found.iter_mut().find(|(k, _)| *k == key) {
            Some((_, suggestion)) => suggestion.assignments.push(*id),
            None => found.push((
                key,
                Suggestion {
                    title: stem,
                    assignments: vec![*id],
                },
            )),
        }
    }
    found
        .into_iter()
        .map(|(_, suggestion)| suggestion)
        .filter(|suggestion| suggestion.assignments.len() >= 2)
        .collect()
}

#[cfg(test)]
mod tests;
