use uuid::Uuid;

/// One audited action. `meta` holds ids and roles only, never personal data (AGENTS §7).
#[derive(Debug, Clone, PartialEq)]
pub struct AuditEntry {
    /// Dotted verb, e.g. `course.publish`.
    pub action: &'static str,
    pub resource_type: &'static str,
    pub resource_id: Uuid,
    pub meta: serde_json::Value,
}
