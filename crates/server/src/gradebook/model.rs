use chrono::{DateTime, Utc};
use sqlx::types::Decimal;
use uuid::Uuid;

use crate::points;
use crate::text_enum::text_enum;

const FEEDBACK_MAX_LEN: usize = 4000;

/// One row of `gradebook.grade_history`: the values after a save or a publish (ADR-0028).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: Uuid,
    /// `save_draft`, `publish` or `auto_score` (a quiz scored by the server, no person).
    pub action: String,
    pub score: Option<Decimal>,
    pub feedback: Option<String>,
    pub actor: Option<HistoryActor>,
    pub at: DateTime<Utc>,
}

/// The published slot of one cell: what the student may read (ADR-0028).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedGrade {
    pub score: Option<Decimal>,
    pub feedback: Option<String>,
    pub published_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryActor {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
}

/// A value that failed validation. Each variant maps to one field of the API.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    /// Not a decimal string with at most two decimals from 0 to the assignment's `max_points`.
    #[error("must be a decimal string from 0 to {max:.2} with at most two decimals")]
    Score { max: Decimal },
    #[error("must be at most {FEEDBACK_MAX_LEN} characters of plain text")]
    Feedback,
}

/// One rejected field of a grade write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldProblem {
    pub field: &'static str,
    pub message: String,
}

/// A valid score: `0 <= score <= max` with at most two decimals. Zero is a score.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Score(Decimal);

impl Score {
    /// `max` is the `max_points` of the assignment, read from its locked row.
    pub fn parse(raw: &str, max: Decimal) -> Result<Self, ModelError> {
        points::parse_decimal(raw)
            .filter(|value| *value <= max)
            .map(Self)
            .ok_or(ModelError::Score { max })
    }

    pub fn value(self) -> Decimal {
        self.0
    }
}

/// Plain text, trimmed, at most 4000 characters. Newlines and tabs are allowed, other control
/// characters are not. Text that is empty after trimming is no feedback at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feedback(String);

impl Feedback {
    pub fn parse(raw: &str) -> Result<Option<Self>, ModelError> {
        let text = raw.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let plain = !text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t' | '\r'));
        if plain && text.chars().count() <= FEEDBACK_MAX_LEN {
            Ok(Some(Self(text.to_owned())))
        } else {
            Err(ModelError::Feedback)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What the client sent for `feedback`. A value that is not text is judged after the actor was
/// authorized, like every other value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedbackInput {
    Text(String),
    NotText,
}

/// The raw draft cell a client sends: `None` clears a field. Nothing in it is trusted before
/// the service has authorized the actor and read `max_points`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GradeInput {
    pub score: Option<String>,
    pub feedback: Option<FeedbackInput>,
}

/// Which students a publish applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Students {
    All,
    Listed(Vec<Uuid>),
}

/// A grade cell as staff see it: both slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub draft_score: Option<Decimal>,
    pub draft_feedback: Option<String>,
    pub published_score: Option<Decimal>,
    pub published_feedback: Option<String>,
    pub has_unpublished_changes: bool,
}

/// The status of the item behind a column, so staff can tell a draft column from a published one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnStatus {
    Draft,
    Published,
}
text_enum!(ColumnStatus { Draft => "draft", Published => "published" });

/// One assignment of the course, in course tree order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub assignment_id: Uuid,
    pub title: String,
    pub max_points: Decimal,
    pub is_bonus: bool,
    pub status: ColumnStatus,
}

/// One student of the gradebook. `cells` holds only the assignments the student has a grade row
/// for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GradebookRow {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
    /// Not on the roster now (left, disabled, promoted, excluded); the grades are kept and read-only
    /// (ADR-0037).
    pub former: bool,
    pub cells: Vec<(Uuid, Cell)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GradebookPage {
    pub columns: Vec<Column>,
    pub rows: Vec<GradebookRow>,
    /// The cursor for the next page, `None` on the last one.
    pub next: Option<Uuid>,
}

/// A published grade as its student sees it. There is no draft field in this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MyGrade {
    pub assignment_id: Uuid,
    pub title: String,
    pub max_points: Decimal,
    pub is_bonus: bool,
    pub score: Option<Decimal>,
    pub feedback: Option<String>,
    pub published_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn max(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    #[test]
    fn a_score_is_zero_up_to_max_with_two_decimals() {
        for ok in ["0", "0.00", "0.01", "7.5", "19.99", "20", "20.00"] {
            assert!(Score::parse(ok, max("20")).is_ok(), "{ok}");
        }
        for bad in [
            "-0.01", "-1", "20.01", "21", "1.005", "1e1", "", " 5", "5 ", "abc", "+5",
        ] {
            assert_eq!(
                Score::parse(bad, max("20")),
                Err(ModelError::Score { max: max("20") }),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_score_message_names_the_max_with_two_decimals() {
        let error = Score::parse("21", max("20")).unwrap_err();
        assert_eq!(
            error.to_string(),
            "must be a decimal string from 0 to 20.00 with at most two decimals"
        );
        let half = Score::parse("13", max("12.5")).unwrap_err();
        assert!(half.to_string().contains("12.50"), "{half}");
    }

    #[test]
    fn feedback_is_trimmed_bounded_plain_text_and_empty_means_none() {
        assert_eq!(
            Feedback::parse("  good  ").unwrap().unwrap().as_str(),
            "good"
        );
        assert_eq!(
            Feedback::parse("line\n\tnext").unwrap().unwrap().as_str(),
            "line\n\tnext"
        );
        for empty in ["", "   ", "\n\t "] {
            assert_eq!(Feedback::parse(empty), Ok(None), "{empty:?}");
        }
        assert!(Feedback::parse(&"ł".repeat(4000)).unwrap().is_some());
        assert_eq!(
            Feedback::parse(&"ł".repeat(4001)),
            Err(ModelError::Feedback)
        );
        for bad in ["a\u{0}b", "a\u{7f}b", "a\u{1b}[0m"] {
            assert_eq!(Feedback::parse(bad), Err(ModelError::Feedback), "{bad:?}");
        }
    }

    #[test]
    fn the_column_status_round_trips() {
        for status in [ColumnStatus::Draft, ColumnStatus::Published] {
            assert_eq!(status.as_str().parse::<ColumnStatus>(), Ok(status));
        }
    }
}
