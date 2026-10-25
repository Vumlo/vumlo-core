use uuid::Uuid;

/// What the administrator filters the audit log by (ADR-0053).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditFilter {
    /// A case-insensitive substring of the actor's name, the action or the label of the target.
    pub text: Option<String>,
    /// One of `accounts`, `owners`, `teachers`, `groups`, `courses`, `content`, `grades`, `terms`
    /// or `other`.
    pub kind: Option<String>,
    pub days: i32,
}

/// One row of the audit log with the names it points at (ADR-0053).
#[derive(Debug, Clone, PartialEq)]
pub struct AuditRow {
    pub id: Uuid,
    pub at: chrono::DateTime<chrono::Utc>,
    pub source: String,
    pub kind: String,
    pub action: String,
    pub actor: Option<(Uuid, String, String)>,
    pub resource_type: String,
    pub resource_id: Uuid,
    pub resource_label: Option<String>,
    pub meta: serde_json::Map<String, serde_json::Value>,
}
