use chrono::{TimeZone, Utc};
use uuid::Uuid;

use super::*;

fn cursor() -> Cursor {
    Cursor {
        at: Utc.timestamp_micros(1_700_000_000_123_456).unwrap(),
        a: Uuid::now_v7(),
        b: Uuid::now_v7(),
    }
}

#[test]
fn a_cursor_reads_back_as_it_was_written_to_the_microsecond() {
    let cursor = cursor();
    let text = cursor.to_string();
    assert_eq!(text.parse::<Cursor>(), Ok(cursor));
    // Nothing a query string would have to escape.
    assert!(
        text.chars()
            .all(|c| c.is_ascii_hexdigit() || c == '-' || c == '.')
    );
}

#[test]
fn a_cursor_that_was_not_written_by_us_is_refused() {
    let good = cursor().to_string();
    for bad in [
        "",
        "junk",
        "1.2",
        "x.00000000-0000-0000-0000-000000000000.00000000-0000-0000-0000-000000000000",
        &format!("{good}.extra"),
        &good.replacen('.', "", 1),
    ] {
        assert!(bad.parse::<Cursor>().is_err(), "{bad}");
    }
}

#[test]
fn a_page_is_between_one_and_a_hundred_and_defaults_to_fifty() {
    assert_eq!(PluginPage::new(None, None).unwrap().limit, 50);
    assert_eq!(PluginPage::new(None, Some(0)).unwrap().limit, 1);
    assert_eq!(PluginPage::new(None, Some(10_000)).unwrap().limit, 100);
    assert!(PluginPage::new(Some("junk"), None).is_err());
    let good = cursor();
    assert_eq!(
        PluginPage::new(Some(&good.to_string()), Some(7))
            .unwrap()
            .after,
        Some(good)
    );
}

#[test]
fn the_extra_row_only_proves_there_is_more() {
    let key = |n: &u32| Cursor::by_id(Uuid::from_u128(u128::from(*n)));
    let more = PluginList::from_rows(vec![1, 2, 3], 2, key);
    assert_eq!(more.items, [1, 2]);
    assert_eq!(more.next, Some(Cursor::by_id(Uuid::from_u128(2))));
    let last = PluginList::from_rows(vec![1, 2], 2, key);
    assert_eq!(last.items, [1, 2]);
    assert_eq!(last.next, None);
}
