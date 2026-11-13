use uuid::Uuid;

/// How many regular courses use a term, and where today stands against it (ADR-0053).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermUsage {
    pub term_id: Uuid,
    pub courses: i64,
    pub current: bool,
    pub ended: bool,
}
