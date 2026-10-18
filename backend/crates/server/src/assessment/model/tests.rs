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
