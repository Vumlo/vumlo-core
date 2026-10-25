//! Request and response types of the plugin API and of the reports it stores (ADR-0066).

use chrono::{DateTime, Utc};
use garde::Validate;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::ToSchema;
use uuid::Uuid;

/// The most a report's `data` may take when written as JSON.
pub const MAX_REPORT_DATA_BYTES: usize = 16 * 1024;

/// A submitted attempt as a plugin with `submissions:read` sees it: which work, whose, and what it
/// says. No draft, no score and no grade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PluginSubmission {
    /// The id the `submission.submitted` event carries as `submission_id`.
    pub id: Uuid,
    pub assignment_id: Uuid,
    pub student_id: Uuid,
    pub attempt: i32,
    /// A block document for a `text` assignment, `{"answers": {...}}` for a quiz or a form.
    #[schema(value_type = Object)]
    pub content: Map<String, Value>,
    pub submitted_at: DateTime<Utc>,
}

/// What a plugin says about one submitted attempt. It replaces the plugin's earlier report on it.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct PutReportRequest {
    /// One line for people to read, 1 to 500 characters.
    #[garde(length(min = 1, max = 500))]
    pub summary: String,
    /// What a program may want: a JSON object of at most 16 KiB.
    #[schema(value_type = Object)]
    #[garde(custom(a_small_object))]
    pub data: Value,
}

fn a_small_object(data: &Value, _: &()) -> garde::Result {
    if !data.is_object() {
        return Err(garde::Error::new("must be a JSON object"));
    }
    if data.to_string().len() > MAX_REPORT_DATA_BYTES {
        return Err(garde::Error::new(format!(
            "must take at most {MAX_REPORT_DATA_BYTES} bytes as JSON"
        )));
    }
    Ok(())
}

/// The report of one plugin on one submitted attempt, for the staff who may see the attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PluginReport {
    /// The id the plugin gave in its manifest.
    pub plugin_id: String,
    pub summary: String,
    #[schema(value_type = Object)]
    pub data: Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PluginReports {
    pub items: Vec<PluginReport>,
}

#[cfg(test)]
mod tests;
