use crate::content::NextDeadline;
use crate::gradebook::Progress;
use crate::teaching::CardCourse;

/// One entry of a staff card's to-do list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Todo {
    ToGrade { count: i64 },
    DraftGrades { count: i64, auto_scored: bool },
    UnpublishedItems { count: i64, titles: Vec<String> },
    NextDeadline(NextDeadline),
}

/// The card of one course, whichever side of it the caller is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CourseCardData {
    pub course: CardCourse,
    pub owner: bool,
    pub progress: Option<Progress>,
    pub next: Option<NextDeadline>,
    pub todo: Vec<Todo>,
}
