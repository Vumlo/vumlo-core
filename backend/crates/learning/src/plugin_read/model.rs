//! What a plugin is shown. Plain domain types: the HTTP edge maps them to DTOs.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, TimeZone, Utc};
use common::InvalidCursor;
use serde_json::{Map, Value};
use uuid::Uuid;

const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 100;

/// Where a list stopped: the sort key of its last item, `(time, a, b)`. A list sorted by something
/// that is not unique adds ids to make it so; a list whose key has one id leaves `b` nil. It reads
/// `<microseconds>.<a>.<b>`, which a query string carries as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub at: DateTime<Utc>,
    pub a: Uuid,
    pub b: Uuid,
}

impl Cursor {
    /// The sort key of an item whose list has no time: the roster, sorted by user.
    pub(crate) fn by_id(id: Uuid) -> Self {
        Self {
            at: DateTime::UNIX_EPOCH,
            a: id,
            b: Uuid::nil(),
        }
    }
}

impl fmt::Display for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.at.timestamp_micros(), self.a, self.b)
    }
}

impl FromStr for Cursor {
    type Err = InvalidCursor;

    fn from_str(raw: &str) -> Result<Self, InvalidCursor> {
        let mut parts = raw.split('.');
        let (Some(micros), Some(a), Some(b), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(InvalidCursor);
        };
        Ok(Self {
            at: Utc
                .timestamp_micros(micros.parse().map_err(|_| InvalidCursor)?)
                .single()
                .ok_or(InvalidCursor)?,
            a: Uuid::parse_str(a).map_err(|_| InvalidCursor)?,
            b: Uuid::parse_str(b).map_err(|_| InvalidCursor)?,
        })
    }
}

/// Which page of a list to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginPage {
    pub after: Option<Cursor>,
    pub limit: u32,
}

impl PluginPage {
    pub fn new(cursor: Option<&str>, limit: Option<u32>) -> Result<Self, InvalidCursor> {
        Ok(Self {
            after: cursor.map(str::parse).transpose()?,
            limit: limit.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginList<T> {
    pub items: Vec<T>,
    pub next: Option<Cursor>,
}

impl<T> PluginList<T> {
    /// Builds a page from rows fetched with `limit + 1`: the extra row only proves there is more.
    pub(crate) fn from_rows(mut rows: Vec<T>, limit: u32, key: impl Fn(&T) -> Cursor) -> Self {
        let limit = limit as usize;
        let more = rows.len() > limit;
        rows.truncate(limit);
        let next = if more { rows.last().map(key) } else { None };
        Self { items: rows, next }
    }
}

/// An assignment as a student could see it: open, with its published title.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginAssignment {
    pub id: Uuid,
    pub course_id: Uuid,
    /// The assignment type: `text`, `no_submission`, `quiz`, `form` or `file`.
    pub kind: String,
    pub title: String,
    pub due_at: Option<DateTime<Utc>>,
    pub max_points: String,
    /// The field definitions of a `form`; `None` for any other type.
    pub fields: Option<Vec<Value>>,
    /// When the assignment, its title or its settings last changed.
    pub updated_at: DateTime<Utc>,
}

/// A submitted attempt as a plugin with `submissions:read` sees it: which work, whose, and what it
/// says. No draft, no score and no grade.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginSubmission {
    pub id: Uuid,
    pub course_id: Uuid,
    pub assignment_id: Uuid,
    pub student_id: Uuid,
    pub attempt: i32,
    pub content: Map<String, Value>,
    pub submitted_at: DateTime<Utc>,
}

/// The values a student gave in a `form`, by field id.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginFormAnswers {
    pub submission_id: Uuid,
    pub course_id: Uuid,
    pub assignment_id: Uuid,
    pub student_id: Uuid,
    pub answers: Map<String, Value>,
}

/// One submitted attempt in the list of an assignment, without its content.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginSubmissionRow {
    pub id: Uuid,
    pub student_id: Uuid,
    pub attempt: i32,
    pub submitted_at: DateTime<Utc>,
    pub is_late: bool,
    /// When the grade of this student for the assignment was published, if it was.
    pub graded_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginMember {
    pub user_id: Uuid,
    pub name: String,
    pub surname: String,
    /// `teacher`, `assistant` or `student`.
    pub role: String,
    /// `direct` or `group`.
    pub source: String,
}

/// A published grade. The score and the feedback of a draft are not here.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginGrade {
    pub course_id: Uuid,
    pub assignment_id: Uuid,
    pub student_id: Uuid,
    pub score: Option<String>,
    pub max_points: String,
    pub published_by: Option<Uuid>,
    pub published_at: DateTime<Utc>,
}

/// The questions of a quiz without the answer key.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginQuiz {
    pub assignment_id: Uuid,
    pub course_id: Uuid,
    pub max_points: String,
    pub config: Map<String, Value>,
}

/// A submitted attempt at a quiz with its score per question.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginAttempt {
    pub id: Uuid,
    pub student_id: Uuid,
    pub attempt: i32,
    pub scores: Map<String, Value>,
    pub started_at: DateTime<Utc>,
    pub submitted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginContact {
    pub user_id: Uuid,
    pub email: String,
}
