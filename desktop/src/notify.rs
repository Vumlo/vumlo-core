//! Deadline notifications (ADR-0056): a student is told a day and an hour before work is due, once each. The
//! operating system's notification plugin cannot schedule a notification for later, so the app checks every
//! quarter of an hour while it runs (it stays running in the tray).

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::core::{ApiRequest, Core, now_ms};

const CHECK_EVERY: Duration = Duration::from_secs(15 * 60);
const HOUR: i64 = 3600;
const DAY: i64 = 24 * HOUR;

#[derive(Debug, PartialEq)]
pub struct Notice {
    pub assignment_id: String,
    pub title: String,
    pub due_at: String,
    /// `day` (less than a day to go) or `hour` (less than an hour).
    pub window: &'static str,
}

/// Unix seconds of `2026-10-30T20:59:00Z` (a fraction of a second and a `+03:00` offset are accepted).
pub fn parse_rfc3339(text: &str) -> Option<i64> {
    let (date, time) = text.split_once('T')?;
    let mut ymd = date.split('-').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (ymd.next()??, ymd.next()??, ymd.next()??);
    let split = time.find(['Z', '+', '-'])?;
    let (clock, zone) = time.split_at(split);
    let mut hms = clock.split(':');
    let hour: i64 = hms.next()?.parse().ok()?;
    let minute: i64 = hms.next()?.parse().ok()?;
    let second: i64 = hms.next().unwrap_or("0").split('.').next()?.parse().ok()?;
    let offset = match zone {
        "Z" => 0,
        zone => {
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let (h, m) = zone[1..].split_once(':')?;
            sign * (h.parse::<i64>().ok()? * HOUR + m.parse::<i64>().ok()? * 60)
        }
    };
    // Civil date to days since 1970-01-01 (the inverse of `core::rfc3339`).
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let year_of_era = y.rem_euclid(400);
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(days * 86_400 + hour * HOUR + minute * 60 + second - offset)
}

/// The notices due now among the items of `/me/work`: work not yet turned in, due within a day and not past.
pub fn due_notices(work: &Value, now_s: i64) -> Vec<Notice> {
    let Some(items) = work.get("items").and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let status = item.get("status")?.as_str()?;
            if matches!(status, "submitted" | "graded") {
                return None;
            }
            let due_at = item.get("due_at")?.as_str()?;
            let left = parse_rfc3339(due_at)? - now_s;
            let window = match left {
                l if l <= 0 || l > DAY => return None,
                l if l <= HOUR => "hour",
                _ => "day",
            };
            Some(Notice {
                assignment_id: item.get("assignment_id")?.as_str()?.to_owned(),
                title: item.get("title")?.as_str()?.to_owned(),
                due_at: due_at.to_owned(),
                window,
            })
        })
        .collect()
}

/// The words of a notice in the language of the window.
pub fn words(language: &str, window: &str) -> &'static str {
    match (language, window) {
        ("uk", "hour") => "Дедлайн за годину",
        ("uk", _) => "Дедлайн протягом доби",
        (_, "hour") => "Due within an hour",
        (_, _) => "Due within a day",
    }
}

async fn check(core: &Core, app: &AppHandle) {
    if !core.signed_in() {
        return;
    }
    // Online this refreshes the copy; offline it is the copy, which is what the person sees too.
    let Ok(reply) = core
        .request(ApiRequest {
            method: "GET".into(),
            path: "/me/work".into(),
            ..Default::default()
        })
        .await
    else {
        return;
    };
    let Some(work) = reply.body else { return };
    let language = core.language();
    for notice in due_notices(&work, now_ms() / 1000) {
        if core.notice_new(&notice.assignment_id, &notice.due_at, notice.window) {
            let _ = app
                .notification()
                .builder()
                .title(words(&language, notice.window))
                .body(notice.title)
                .show();
        }
    }
}

pub async fn run(core: Arc<Core>, app: AppHandle) {
    loop {
        check(&core, &app).await;
        tokio::time::sleep(CHECK_EVERY).await;
    }
}

#[cfg(test)]
mod tests;
