//! The registry is global to the process, so these tests need a process of their own.

use events::{EventsError, Subscribers, install};

#[test]
fn install_twice_is_an_error() {
    install(Subscribers::new()).expect("the first install works");

    let second = install(Subscribers::new());

    assert!(matches!(second, Err(EventsError::AlreadyInstalled)));
}
