use super::*;

fn store() -> Store {
    Store::open_in_memory().unwrap()
}

const DRAFT: &str = "/assignments/a1/submission/draft";
const SUBMIT: &str = "/assignments/a1/submission/submit";

#[test]
fn a_cached_answer_is_replaced_touched_and_dropped() {
    let s = store();
    assert!(s.cache_get("/me").unwrap().is_none());
    s.cache_put("/me", "{\"a\":1}", Some("W/\"1\""), 100)
        .unwrap();
    s.cache_put("/me", "{\"a\":2}", Some("W/\"2\""), 200)
        .unwrap();
    let hit = s.cache_get("/me").unwrap().unwrap();
    assert_eq!(
        (hit.body.as_str(), hit.etag.as_deref(), hit.fetched_at),
        ("{\"a\":2}", Some("W/\"2\""), 200)
    );
    s.cache_touch("/me", 300).unwrap();
    assert_eq!(s.cache_get("/me").unwrap().unwrap().fetched_at, 300);
    s.cache_delete("/me").unwrap();
    assert!(s.cache_get("/me").unwrap().is_none());
}

#[test]
fn rows_leave_in_the_order_they_came_and_wait_for_their_time() {
    let s = store();
    let first = s.enqueue("POST", "/a", None, "k1", 1).unwrap();
    let second = s.enqueue("POST", "/b", None, "k2", 2).unwrap();
    assert_eq!(s.next_due(10).unwrap().unwrap().id, first);
    s.mark_retry(first, 1, 100).unwrap();
    // The first waits for its back-off; the next row is not held up behind it.
    assert_eq!(s.next_due(10).unwrap().unwrap().id, second);
    assert_eq!(s.next_wake().unwrap(), Some(0));
    s.mark_sending(second).unwrap();
    assert!(
        s.next_due(10).unwrap().is_none(),
        "a row being sent is not due"
    );
    assert_eq!(s.next_wake().unwrap(), Some(100));
    assert!(s.next_due(100).unwrap().is_some());
    s.retry_all_now().unwrap();
    assert_eq!(s.next_due(0).unwrap().unwrap().id, first);
}

#[test]
fn a_newer_draft_replaces_the_waiting_one_and_keeps_its_place() {
    let s = store();
    let first = s.enqueue("PUT", DRAFT, Some("v1"), "k1", 1).unwrap();
    s.enqueue(
        "PUT",
        "/assignments/other/submission/draft",
        Some("x"),
        "kx",
        2,
    )
    .unwrap();
    let again = s.enqueue("PUT", DRAFT, Some("v2"), "k2", 3).unwrap();
    assert_eq!(again, first, "the same row");
    let rows = s.outbox_list().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (
            rows[0].body.as_deref(),
            rows[0].idem_key.as_str(),
            rows[0].created_at
        ),
        (Some("v2"), "k2", 3)
    );
}

#[test]
fn a_draft_edited_after_a_queued_submit_is_a_new_row() {
    let s = store();
    s.enqueue("PUT", DRAFT, Some("v1"), "k1", 1).unwrap();
    s.enqueue("POST", SUBMIT, None, "k2", 2).unwrap();
    s.enqueue("PUT", DRAFT, Some("v2"), "k3", 3).unwrap();
    let rows = s.outbox_list().unwrap();
    let order: Vec<_> = rows
        .iter()
        .map(|r| (r.method.as_str(), r.body.as_deref()))
        .collect();
    assert_eq!(
        order,
        [("PUT", Some("v1")), ("POST", None), ("PUT", Some("v2"))],
        "v1 is what the submit sends"
    );
}

#[test]
fn a_draft_that_is_being_sent_is_not_overwritten() {
    let s = store();
    let id = s.enqueue("PUT", DRAFT, Some("v1"), "k1", 1).unwrap();
    s.mark_sending(id).unwrap();
    let next = s.enqueue("PUT", DRAFT, Some("v2"), "k2", 2).unwrap();
    assert_ne!(next, id);
    assert_eq!(s.outbox_list().unwrap().len(), 2);
}

#[test]
fn a_row_that_failed_for_good_stays_until_it_is_discarded_and_blocks_nothing() {
    let s = store();
    let bad = s.enqueue("POST", SUBMIT, None, "k1", 1).unwrap();
    let good = s.enqueue("POST", "/c", None, "k2", 2).unwrap();
    s.mark_failed(
        bad,
        409,
        Some("{\"detail\":\"the submission window is closed\"}"),
    )
    .unwrap();
    assert_eq!(s.next_due(0).unwrap().unwrap().id, good);
    let rows = s.outbox_list().unwrap();
    assert_eq!(
        (rows[0].state.as_str(), rows[0].status),
        ("failed", Some(409))
    );
    assert!(s.has_waiting().unwrap());
    s.outbox_delete(good).unwrap();
    assert!(!s.has_waiting().unwrap(), "a failed row is not waiting");
    s.outbox_delete(bad).unwrap();
    assert!(s.outbox_list().unwrap().is_empty());
}

#[test]
fn leaving_wipes_what_was_read_and_what_was_not_sent() {
    let s = store();
    s.cache_put("/me", "{}", None, 1).unwrap();
    s.enqueue("POST", SUBMIT, None, "k", 1).unwrap();
    s.meta_set("user_id", "u1").unwrap();
    s.meta_set("language", "uk").unwrap();
    assert!(
        s.notified_insert("a1", "2026-10-30T20:59:00Z", "hour")
            .unwrap()
    );
    assert_eq!(s.meta_get("user_id").unwrap().as_deref(), Some("u1"));
    s.wipe().unwrap();
    assert_eq!(
        s.meta_get("language").unwrap().as_deref(),
        Some("uk"),
        "the language belongs to the device"
    );
    assert!(
        s.notified_insert("a1", "2026-10-30T20:59:00Z", "hour")
            .unwrap(),
        "a new person is told again"
    );
    assert!(s.cache_get("/me").unwrap().is_none());
    assert!(s.outbox_list().unwrap().is_empty());
    assert!(s.meta_get("user_id").unwrap().is_none());
}

#[test]
fn a_row_that_was_being_sent_when_the_app_stopped_is_sent_again() {
    let dir = std::env::temp_dir().join(format!("vumlo-store-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("offline.sqlite3");
    let _ = std::fs::remove_file(&path);
    {
        let s = Store::open(&path).unwrap();
        let id = s.enqueue("POST", SUBMIT, None, "k", 1).unwrap();
        s.mark_sending(id).unwrap();
    }
    let reopened = Store::open(&path).unwrap();
    assert_eq!(reopened.outbox_list().unwrap()[0].state, "pending");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_notice_is_recorded_once_per_deadline_and_window() {
    let s = store();
    assert!(
        s.notified_insert("a1", "2026-10-30T20:59:00Z", "day")
            .unwrap()
    );
    assert!(
        !s.notified_insert("a1", "2026-10-30T20:59:00Z", "day")
            .unwrap()
    );
    assert!(
        s.notified_insert("a1", "2026-10-30T20:59:00Z", "hour")
            .unwrap(),
        "the hour is its own notice"
    );
    assert!(
        s.notified_insert("a1", "2026-11-02T20:59:00Z", "day")
            .unwrap(),
        "a moved deadline is told again"
    );
}
