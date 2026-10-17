use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::content::SubmissionSettings;
use crate::pagination::Page;

/// The unsubmitted document of a student for an assignment (ADR-0011). Only its owner reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub content: Map<String, Value>,
    pub updated_at: DateTime<Utc>,
}

/// A submitted attempt. It never changes (ADR-0011, ADR-0035).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    pub id: Uuid,
    pub attempt: i32,
    pub content: Map<String, Value>,
    pub submitted_at: DateTime<Utc>,
    pub is_late: bool,
}

/// A student's own work: the draft, if any, and a page of attempts, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MySubmission {
    pub draft: Option<Draft>,
    pub attempts: Page<Attempt>,
}

/// The latest submitted attempt of a student, without its content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestAttempt {
    pub id: Uuid,
    pub attempt: i32,
    pub submitted_at: DateTime<Utc>,
    pub is_late: bool,
}

/// A student of the course and their latest attempt, if there is one (staff list). A former
/// student is one who left the roster but has an attempt (ADR-0037).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudentSubmission {
    pub student_id: Uuid,
    pub name: String,
    pub surname: String,
    pub former: bool,
    pub latest: Option<LatestAttempt>,
}

/// Whether a save or a submit is taken at some moment (ADR-0035 point 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    /// Taken; `late` when it is after `due_at`.
    Open {
        late: bool,
    },
    Closed,
}

impl Window {
    /// The window at `now`. It closes right after `due_at` when late work is not accepted, and
    /// right after `late_until` (never, when unset) when it is; a moment equal to either bound is
    /// still inside. Without a deadline it never closes. The settings come from the locked
    /// assignment row, the time from the caller.
    pub fn at(settings: &SubmissionSettings, now: DateTime<Utc>) -> Self {
        let Some(due_at) = settings.due_at else {
            return Self::Open { late: false };
        };
        if now <= due_at {
            return Self::Open { late: false };
        }
        match (settings.accept_late, settings.late_until) {
            (Some(true), None) => Self::Open { late: true },
            (Some(true), Some(until)) if now <= until => Self::Open { late: true },
            _ => Self::Closed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::OnDeadline;

    fn at(hour: u32) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(&format!("2030-05-01T{hour:02}:00:00Z"))
            .unwrap()
            .with_timezone(&Utc)
    }

    fn settings(accept_late: bool, late_until: Option<DateTime<Utc>>) -> SubmissionSettings {
        SubmissionSettings {
            due_at: Some(at(10)),
            on_deadline: Some(OnDeadline::KeepDraft),
            accept_late: Some(accept_late),
            late_until,
            max_attempts: None,
        }
    }

    #[test]
    fn without_a_deadline_the_window_is_always_open_and_never_late() {
        let none = SubmissionSettings::default();
        for hour in [0, 12, 23] {
            assert_eq!(Window::at(&none, at(hour)), Window::Open { late: false });
        }
    }

    #[test]
    fn work_is_on_time_up_to_and_including_the_deadline() {
        for accept in [false, true] {
            let s = settings(accept, None);
            assert_eq!(Window::at(&s, at(9)), Window::Open { late: false });
            assert_eq!(Window::at(&s, at(10)), Window::Open { late: false });
        }
    }

    #[test]
    fn after_the_deadline_the_window_is_closed_unless_late_work_is_accepted() {
        let strict = settings(false, None);
        assert_eq!(Window::at(&strict, at(11)), Window::Closed);
        let just_after = at(10) + chrono::Duration::milliseconds(1);
        assert_eq!(Window::at(&strict, just_after), Window::Closed);
    }

    #[test]
    fn late_work_is_taken_until_late_until_and_for_ever_without_it() {
        let open_ended = settings(true, None);
        assert_eq!(Window::at(&open_ended, at(11)), Window::Open { late: true });
        assert_eq!(
            Window::at(&open_ended, at(10) + chrono::Duration::days(3650)),
            Window::Open { late: true }
        );
        let bounded = settings(true, Some(at(12)));
        assert_eq!(Window::at(&bounded, at(11)), Window::Open { late: true });
        assert_eq!(Window::at(&bounded, at(12)), Window::Open { late: true });
        let after = at(12) + chrono::Duration::milliseconds(1);
        assert_eq!(Window::at(&bounded, after), Window::Closed);
    }
}
