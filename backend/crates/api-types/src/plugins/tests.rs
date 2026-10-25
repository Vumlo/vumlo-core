use serde_json::json;

use super::*;

fn request(summary: &str, data: Value) -> PutReportRequest {
    PutReportRequest {
        summary: summary.to_owned(),
        data,
    }
}

#[test]
fn a_report_has_a_line_to_read_and_a_small_object() {
    assert!(request("12 words", json!({"words": 12})).validate().is_ok());
    assert!(request("x", json!({})).validate().is_ok());
    assert!(request("", json!({})).validate().is_err());
    assert!(request(&"x".repeat(501), json!({})).validate().is_err());
    assert!(request("ok", json!([1, 2])).validate().is_err());
    assert!(request("ok", json!("text")).validate().is_err());
    let big = json!({"blob": "x".repeat(MAX_REPORT_DATA_BYTES)});
    assert!(request("ok", big).validate().is_err());
}
